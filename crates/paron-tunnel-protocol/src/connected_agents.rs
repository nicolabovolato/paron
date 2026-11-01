//! Which agents the broker can reach right now, and how (RFC §6.1, POL-3, STM-11, STM-12).
//!
//! One connection per agent. When an agent connects again, the new connection takes the old one's
//! place and the old one is ended — that is POL-3 and STM-11, and it is all
//! [`ConnectedAgents::connect`] does.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::error::TunnelError;
use crate::identity::AgentId;
use crate::transport::Stream;
use crate::wire::Selector;

/// A way to reach one agent. Implemented by whatever is driving that agent's connection.
#[async_trait]
pub trait Tunnel: Send + Sync {
    /// Open a stream to whatever `selector` names on the agent's host.
    ///
    /// `Refused` means two things at once — the agent said no (POL-8), or the open took too long
    /// and we gave up (TRN-6). §6.1 wants a consumer to see the same answer either way, so they
    /// are not separated here.
    async fn open(&self, selector: &Selector) -> Result<Box<dyn Stream>, TunnelError>;

    /// End this connection now.
    ///
    /// Called when a newer connection for the same agent replaces this one. STM-11 says to end it
    /// immediately rather than leave it hanging around until the liveness probes eventually notice
    /// it is redundant.
    async fn close(&self);
}

/// Which of an agent's connections this one is: first, second, third.
///
/// Every connection gets the next number when it goes in the map. The point is teardown ordering:
/// a connection being cleaned up can check whether the entry under its agent is still *its* entry,
/// or whether a newer connection has taken the slot, in which case it leaves it alone (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Generation(u64);

/// The connection currently reaching one agent.
struct OpenConnection {
    generation: Generation,
    tunnel: Arc<dyn Tunnel>,
}

/// Every agent the broker can currently reach, and the connection to reach it over.
///
/// This is what the RFC calls the registration table (§6.1). It sits outside both state machines
/// because both ends of the broker need it: the code driving a connection adds an agent when it is
/// admitted, and whatever wants to reach that agent looks it up here. One of these serves a whole
/// broker however many ports it listens on, which is how STM-11's "one connection per agent" holds
/// across both transport mappings at once (STM-18).
///
/// It is also the answer to STM-4, which says a consumer must not be handed a connection before
/// the broker has admitted it. Nothing can be: the only way in is
/// [`connect`](ConnectedAgents::connect), and the broker calls that before it sends the
/// `ACCESS`(Granted) that admits the connection.
#[derive(Default)]
pub struct ConnectedAgents {
    open: Mutex<Open>,
    connected: Notify,
}

/// One lock over both the map and the counter that stamps it.
///
/// The counter is a plain `u64` rather than an `AtomicU64` precisely because it lives here: an
/// atomic would let a connection take its number outside the lock, and then two connections could
/// be numbered in one order and inserted in the other. Under the lock, a connection's number and
/// its place in the map are decided in the same breath, so the numbers say what they look like they
/// say. An atomic would also be the second thing to reason about where one lock already covers
/// everything.
#[derive(Default)]
struct Open {
    by_agent: HashMap<AgentId, OpenConnection>,
    next_generation: u64,
}

impl ConnectedAgents {
    pub fn new() -> Self {
        Self::default()
    }

    /// This connection is now how we reach `agent`. Any earlier one is ended.
    ///
    /// This is POL-3, and the broker calls it at admission, just before it sends
    /// `ACCESS`(Granted) (§6.1, STM-11).
    ///
    /// Why the new connection wins instead of being rejected: getting admitted means proving you
    /// hold the agent's private key, and if two connections both prove it there is nothing to
    /// choose between them. In practice this is almost always the same agent redialling after its
    /// IP changed, with the broker not yet having noticed the old connection is dead. Keeping the
    /// old one would lock the real agent out for a full detection bound (§6.2).
    ///
    /// Hold onto what comes back. Dropping it disconnects the agent.
    pub async fn connect(self: &Arc<Self>, agent: AgentId, tunnel: Arc<dyn Tunnel>) -> Connected {
        let (generation, replaced) = {
            let mut open = self.open.lock();
            open.next_generation += 1;

            let generation = Generation(open.next_generation);
            let replaced = open
                .by_agent
                .insert(agent, OpenConnection { generation, tunnel });

            (generation, replaced)
        };
        self.connected.notify_waiters();

        if let Some(replaced) = replaced {
            // STM-12 requires this to reach an operator, and it is worth understanding why.
            // If someone steals an agent's private key and runs a second agent with it, the two
            // take turns replacing each other, and this line is the only place that shows up
            // (SEC-6 explains why nothing tells the agents themselves). It is a warning rather
            // than info because the innocent explanation — one redial after a network change —
            // happens once, whereas the bad one repeats.
            tracing::warn!(%agent, "agent connection closed: another connection coming from the same agent was opened");

            replaced.tunnel.close().await;
        }

        Connected {
            agents: Arc::clone(self),
            agent,
            generation,
        }
    }

    /// Cut an agent off now, ending its connection.
    ///
    /// This is what an operator's "remove this machine" reaches. The only other way a connection
    /// ends is by dropping its [`Connected`], and that lives deep inside the code driving the
    /// connection, where nobody making an operational decision can get at it.
    ///
    /// **It ends the connection and nothing else.** POL-5 has two halves — unbind the agent's key
    /// so it cannot come back, and end any connection it currently has — and only the second half
    /// is this crate's. Do both, in this order:
    ///
    /// 1. unbind the key, wherever your broker keeps its records;
    /// 2. call this.
    ///
    /// That order matters. Unbind first and an agent that redials in the gap is denied. Call this
    /// first and it simply reconnects on the key it still has, a second later.
    ///
    /// Freezing is the one withdrawal you do not drive: a broker answered Stale invokes
    /// [`BrokerPolicy::unassign_identity`](crate::BrokerPolicy::unassign_identity) and then calls
    /// this itself (§4.2).
    pub async fn disconnect(&self, agent: AgentId) {
        let dropped = self.open.lock().by_agent.remove(&agent);

        if let Some(dropped) = dropped {
            tracing::info!(%agent, "agent connection closed");

            dropped.tunnel.close().await;
        }
    }

    /// The consumer surface: reach `agent` on `selector`. Authorising the caller is outside this
    /// crate (§12.3), so whoever holds this reaches every registered agent.
    ///
    /// §2.5's rule that a preamble names one service is the type of `selector`, so nothing is
    /// checked for it here and a set-naming type cannot reach the wire.
    pub async fn open(
        &self,
        agent: AgentId,
        selector: &Selector,
    ) -> Result<Box<dyn Stream>, TunnelError> {
        self.tunnel_to(agent)
            .ok_or(TunnelError::NotConnected)?
            .open(selector)
            .await
    }

    /// The connection currently reaching `agent`, if there is one.
    pub fn tunnel_to(&self, agent: AgentId) -> Option<Arc<dyn Tunnel>> {
        self.open
            .lock()
            .by_agent
            .get(&agent)
            .map(|open| Arc::clone(&open.tunnel))
    }

    /// Wait until a connection reaches `agent`, then return that connection.
    pub async fn wait_for_tunnel_to(&self, agent: AgentId) -> Arc<dyn Tunnel> {
        loop {
            let connected = self.connected.notified();
            tokio::pin!(connected);
            connected.as_mut().enable();

            if let Some(tunnel) = self.tunnel_to(agent) {
                return tunnel;
            }

            connected.await;
        }
    }

    /// Remove an agent's connection, but only if it is still the one that asked.
    ///
    /// A connection being torn down just after its replacement arrived would otherwise take the
    /// replacement out with it, leaving the agent unreachable while a perfectly good connection
    /// sat there (§6.1).
    fn remove_if_current(&self, agent: AgentId, generation: Generation) {
        let mut open = self.open.lock();

        if open
            .by_agent
            .get(&agent)
            .is_some_and(|connection| connection.generation == generation)
        {
            open.by_agent.remove(&agent);
        }
    }
}

/// Keeps an agent connected for as long as it is held.
///
/// Drop it and the agent is no longer reachable, which is how the RFC's registration ends (§6.1,
/// STM-5). There is nothing to call on it: the code driving a connection keeps one alive alongside
/// that connection, and both go together.
pub struct Connected {
    agents: Arc<ConnectedAgents>,
    agent: AgentId,
    generation: Generation,
}

impl Drop for Connected {
    fn drop(&mut self) {
        self.agents.remove_if_current(self.agent, self.generation);
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll, Waker};

    use super::*;
    use crate::conformance::Reported;

    #[derive(Default)]
    struct RecordingTunnel {
        closed: AtomicBool,
    }

    impl RecordingTunnel {
        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl Tunnel for RecordingTunnel {
        async fn open(&self, _selector: &Selector) -> Result<Box<dyn Stream>, TunnelError> {
            Err(TunnelError::NotConnected)
        }

        async fn close(&self) {
            self.closed.store(true, Ordering::Relaxed);
        }
    }

    fn tunnel() -> Arc<RecordingTunnel> {
        Arc::default()
    }

    fn agent_id() -> AgentId {
        AgentId::new(uuid::Uuid::now_v7())
    }

    fn is_reached_through(
        agents: &ConnectedAgents,
        agent: AgentId,
        tunnel: &Arc<RecordingTunnel>,
    ) -> bool {
        agents
            .tunnel_to(agent)
            .is_some_and(|current| std::ptr::addr_eq(Arc::as_ptr(&current), Arc::as_ptr(tunnel)))
    }

    // `said` holds the output of every test in the process, so only the lines that name this
    // agent count.
    fn lines_about(operator_reports: &Reported, agent: AgentId) -> Vec<String> {
        operator_reports
            .said()
            .into_iter()
            .filter(|line| line.contains(&agent.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn open_without_connection_returns_not_connected() {
        let agents = ConnectedAgents::new();

        let opened = agents.open(agent_id(), &Selector::tcp(22).unwrap()).await;

        assert_matches!(opened.err(), Some(TunnelError::NotConnected));
    }

    #[tokio::test]
    async fn connect_makes_agent_reachable() {
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();
        let tunnel = tunnel();

        let _connection = agents.connect(agent, tunnel.clone()).await;

        assert!(is_reached_through(&agents, agent, &tunnel));
    }

    #[tokio::test]
    async fn wait_for_tunnel_to_then_connect_returns_the_new_tunnel() {
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();
        let tunnel = tunnel();
        let mut context = Context::from_waker(Waker::noop());
        let waiting = agents.wait_for_tunnel_to(agent);
        tokio::pin!(waiting);
        let before_connect = waiting.as_mut().poll(&mut context);

        let _connection = agents.connect(agent, tunnel.clone()).await;

        let Poll::Ready(reached) = waiting.as_mut().poll(&mut context) else {
            panic!("the wait did not end when the agent connected");
        };
        assert!(before_connect.is_pending());
        assert!(std::ptr::addr_eq(
            Arc::as_ptr(&reached),
            Arc::as_ptr(&tunnel)
        ));
    }

    #[tokio::test]
    async fn connect_again_replaces_tunnel() {
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();
        let second_tunnel = tunnel();
        let _first_connection = agents.connect(agent, tunnel()).await;

        let _second_connection = agents.connect(agent, second_tunnel.clone()).await;

        assert!(is_reached_through(&agents, agent, &second_tunnel));
    }

    #[tokio::test]
    async fn connect_again_closes_old_tunnel() {
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();
        let first_tunnel = tunnel();
        let _first_connection = agents.connect(agent, first_tunnel.clone()).await;

        let _second_connection = agents.connect(agent, tunnel()).await;

        assert!(first_tunnel.is_closed());
    }

    #[tokio::test]
    async fn connect_first_time_reports_nothing() {
        let operator_reports = Reported::watching();
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();

        let _connection = agents.connect(agent, tunnel()).await;

        assert_eq!(lines_about(&operator_reports, agent), Vec::<String>::new());
    }

    #[tokio::test]
    async fn connect_again_warns_operator() {
        let operator_reports = Reported::watching();
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();
        let _first_connection = agents.connect(agent, tunnel()).await;

        let _second_connection = agents.connect(agent, tunnel()).await;

        assert_matches!(
            lines_about(&operator_reports, agent).as_slice(),
            [line] if line.starts_with(
                "agent connection closed: another connection coming from the same agent was opened"
            )
        );
    }

    #[tokio::test]
    async fn drop_connected_removes_agent() {
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();
        let connection = agents.connect(agent, tunnel()).await;

        drop(connection);

        assert!(agents.tunnel_to(agent).is_none());
    }

    #[tokio::test]
    async fn drop_replaced_connected_keeps_replacement() {
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();
        let second_tunnel = tunnel();
        let first_connection = agents.connect(agent, tunnel()).await;
        let _second_connection = agents.connect(agent, second_tunnel.clone()).await;

        drop(first_connection);

        assert!(is_reached_through(&agents, agent, &second_tunnel));
    }

    #[tokio::test]
    async fn drop_connected_keeps_other_agent() {
        let agents = Arc::new(ConnectedAgents::new());
        let (kept_agent, dropped_agent) = (agent_id(), agent_id());
        let kept_tunnel = tunnel();
        let _kept_connection = agents.connect(kept_agent, kept_tunnel.clone()).await;
        let dropped_connection = agents.connect(dropped_agent, tunnel()).await;

        drop(dropped_connection);

        assert!(is_reached_through(&agents, kept_agent, &kept_tunnel));
    }

    #[tokio::test]
    async fn disconnect_closes_tunnel() {
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();
        let tunnel = tunnel();
        let _connection = agents.connect(agent, tunnel.clone()).await;

        agents.disconnect(agent).await;

        assert!(tunnel.is_closed());
    }

    #[tokio::test]
    async fn disconnect_removes_agent() {
        let agents = Arc::new(ConnectedAgents::new());
        let agent = agent_id();
        let _connection = agents.connect(agent, tunnel()).await;

        agents.disconnect(agent).await;

        assert!(agents.tunnel_to(agent).is_none());
    }
}
