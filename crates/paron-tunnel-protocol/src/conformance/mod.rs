//! CNF-1 to CNF-23 of RFC §11, run once per transport mapping ([`Suite::quic`] for Appendix A,
//! [`Suite::tcp`] for Appendix B).
//!
//! The tests are behavioural rather than type-level: §11 asks that conformance be a property two
//! implementations in two languages can be compared on, so every member drives the protocol
//! through the same surface an embedder uses.
//!
//! One member per requirement, in the order §11 lists them. What they run against is
//! [`fixtures`].

mod fixtures;
mod occupancy;
mod records;
mod relay;
mod saturation;

use std::sync::Arc;
use std::time::{Duration, Instant};

use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use records::Counting;
pub(crate) use records::Reported;

use crate::broker::DENIAL_DELAY;
use crate::quic::QuicAgentTransport;
use crate::tcp::TcpAgentTransport;
use crate::transport::AgentTransport;
use crate::wire::{self, Access, Frame};
use crate::{
    Agent, AgentId, AgentIdentity, Announced, BrokerIdentity, Identity, InMemoryRollbackIndex,
    PinnedBroker, PortRange, Selector, Selectors, Sequence, TunnelError,
};
use fixtures::{
    Disagreeing, Echo, Relay, TIMEOUT, read_frame_within, short_probes, tcp_ports, wait_until,
    write_claim,
};

/// The suite, bound to one transport mapping.
struct Suite {
    mapping: Mapping,
}

/// Which of the two mappings is under test. It decides the transports, and the packet flavour
/// the severable relay of CNF-8 has to carry.
#[derive(Clone, Copy)]
enum Mapping {
    Quic,
    Tcp,
}

impl Mapping {
    fn agent_transport(self) -> Box<dyn AgentTransport> {
        match self {
            Self::Quic => Box::new(QuicAgentTransport),
            Self::Tcp => Box::new(TcpAgentTransport),
        }
    }
}

impl Suite {
    /// Appendix A.
    fn quic() -> Self {
        Self {
            mapping: Mapping::Quic,
        }
    }

    /// Appendix B.
    fn tcp() -> Self {
        Self {
            mapping: Mapping::Tcp,
        }
    }

    /// **CNF-1.** A joined agent, its served ports, bytes all the way there and back on each.
    ///
    /// Two ports rather than one, and the round trip runs on both: an implementation whose
    /// POL-8 answered only the first would pass a single-port test while everything after it
    /// stayed unreachable.
    async fn cnf1_reaches_offered_services(&self) {
        let (echo, also_echo) = (Echo::tcp().await, Echo::tcp().await);
        let joined = self.joined(tcp_ports([echo.port, also_echo.port])).await;

        for (port, which) in [(echo.port, "the first"), (also_echo.port, "the second")] {
            let mut stream = joined
                .connected_agents()
                .open(joined.id, &Selector::tcp(port).unwrap())
                .await
                .unwrap_or_else(|_| panic!("CNF-1: {which} offered port must be reachable"));

            stream.write_all(b"through-the-tunnel").await.unwrap();
            let mut echoed = [0u8; 18];
            stream.read_exact(&mut echoed).await.unwrap();

            assert_eq!(
                &echoed, b"through-the-tunnel",
                "CNF-1: bytes must round-trip on {which} offered port"
            );
        }
    }

    /// **CNF-2.** POL-8 and §6.1: a selector the agent's POL-8 has no target for is answered
    /// as a refusal, and no data reaches the service behind it. The stream is opened and
    /// refused by the agent; nothing broker-side gates a selector (§3.1).
    async fn cnf2_refuses_selector_never_offered(&self) {
        let echo = Echo::tcp().await;
        let withheld = Echo::tcp().await;
        let joined = self.joined(tcp_ports([echo.port])).await;

        let refused = joined
            .connected_agents()
            .open(joined.id, &Selector::tcp(withheld.port).unwrap())
            .await;

        assert!(
            matches!(refused, Err(TunnelError::Refused)),
            "CNF-2: a selector the agent does not serve must be answered as a refusal"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            withheld.accepted(),
            0,
            "CNF-2: no data may reach a service behind a selector the agent does not serve"
        );
    }

    /// **CNF-3.** POL-2 end to end: a valid token binds the keypair proven on the same
    /// connection; a wrong, foreign or expired token binds nothing and modifies no record; and
    /// repeating the joining procedure succeeds (STM-7).
    async fn cnf3_join_binds_proven_keypair(&self) {
        let broker = self.broker().await;

        let id = AgentId::new(uuid::Uuid::now_v7());
        broker.records.unbound(id, "opensesame");
        let (agent, public) = self.agent(&broker, id, tcp_ports([1]));

        agent
            .join(&SecretString::from("opensesame"))
            .await
            .expect("CNF-3: a valid join token must bind");
        assert_eq!(
            broker.records.bound(id),
            Some(public.clone()),
            "CNF-3: the key bound must be the one proven on the joining connection"
        );

        agent
            .join(&SecretString::from("opensesame"))
            .await
            .expect("CNF-3: repeating the joining procedure must succeed (STM-7)");

        let wrong = AgentId::new(uuid::Uuid::now_v7());
        broker.records.unbound(wrong, "right-token");
        let expired = AgentId::new(uuid::Uuid::now_v7());
        broker.records.expired(expired, "late");
        let foreign = AgentId::new(uuid::Uuid::now_v7());

        let (wrong_token, _) = self.agent(&broker, wrong, tcp_ports([1]));
        let (expired_token, _) = self.agent(&broker, expired, tcp_ports([1]));
        let (another_ids_token, _) = self.agent(&broker, foreign, tcp_ports([1]));

        // Each refusal waits out TRN-10's denial delay, and no one of the three depends on
        // another: together they cost the delay once rather than three times over. Three
        // unadmitted connections from one source is well inside the quarter-of-capacity TRN-10
        // allows, so nothing here is testing the limit by accident.
        tokio::join!(
            async {
                assert!(
                    wrong_token
                        .join(&SecretString::from("wrong-token"))
                        .await
                        .is_err(),
                    "CNF-3: an incorrect token must bind nothing"
                );
            },
            async {
                assert!(
                    expired_token
                        .join(&SecretString::from("late"))
                        .await
                        .is_err(),
                    "CNF-3: an expired token must bind nothing"
                );
            },
            async {
                assert!(
                    another_ids_token
                        .join(&SecretString::from("opensesame"))
                        .await
                        .is_err(),
                    "CNF-3: a token issued for another identifier must bind nothing"
                );
            },
        );

        assert_eq!(broker.records.bound(wrong), None);
        assert_eq!(broker.records.bound(expired), None);
        assert_eq!(broker.records.bound(foreign), None);
    }

    /// **CNF-4.** A claim for an unjoined identifier, and one presenting a keypair other than
    /// the bound one, are each denied, and neither results in a registration.
    async fn cnf4_denies_unjoined_or_wrong_keyed_claim(&self) {
        let broker = self.broker().await;

        let unjoined = AgentId::new(uuid::Uuid::now_v7());
        let (agent, _) = self.agent(&broker, unjoined, tcp_ports([1]));
        assert!(
            matches!(
                agent.serve(std::future::pending()).await,
                Err(TunnelError::Refused)
            ),
            "CNF-4: a claim for an unjoined identifier must be denied"
        );
        assert!(broker.connected_agents().tunnel_to(unjoined).is_none());

        let wrong_key = AgentId::new(uuid::Uuid::now_v7());
        broker
            .records
            .joined(wrong_key, Identity::generate().unwrap().public_key());
        let (agent, _) = self.agent(&broker, wrong_key, tcp_ports([1]));
        assert!(
            matches!(
                agent.serve(std::future::pending()).await,
                Err(TunnelError::Refused)
            ),
            "CNF-4: a keypair other than the bound one must be denied"
        );
        assert!(broker.connected_agents().tunnel_to(wrong_key).is_none());
    }

    /// **CNF-5.** TRN-2: a broker presenting an identity other than the pinned one causes the
    /// agent to abort the handshake before transmitting any protocol message, which is why the
    /// policy is never consulted.
    async fn cnf5_agent_refuses_unpinned_broker(&self) {
        let broker = self.broker().await;

        let id = AgentId::new(uuid::Uuid::now_v7());
        let identity = Identity::generate().unwrap();
        broker.records.joined(id, identity.public_key());

        let unpinned = BrokerIdentity::generate()
            .unwrap()
            .root_public_key()
            .fingerprint();
        let agent = Agent::new(
            vec![self.mapping.agent_transport()],
            PinnedBroker {
                address: broker.address.clone(),
                root: unpinned,
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity { id, keys: identity },
            tcp_ports([1]),
            Counting::new(),
            short_probes(),
        );

        assert!(
            agent.serve(std::future::pending()).await.is_err(),
            "CNF-5: an unpinned broker must be refused"
        );
        assert_eq!(
            broker.records.consulted(),
            0,
            "CNF-5: the agent must abort before anything reaches the broker's policy"
        );
    }

    /// **CNF-6.** STM-11: a replacement registration takes effect immediately, and release of
    /// the registration it replaced does not remove it.
    async fn cnf6_replacement_wins(&self) {
        // Long enough for either mapping to have finished releasing the connection that was
        // replaced, so the assertion below spans the release rather than racing it.
        const AFTER_THE_RELEASE: Duration = Duration::from_millis(200);

        let echo = Echo::tcp().await;
        let joined = self.joined(tcp_ports([echo.port])).await;

        // The same identity and key on a second connection: an agent redialling after a change
        // of network path. The sequence continues from the same counter, as the real agent's
        // would (POL-10).
        let identity = Identity::from_pkcs8_der(&joined.key_der).unwrap();
        let second = Arc::new(Agent::new(
            vec![self.mapping.agent_transport()],
            PinnedBroker {
                address: joined.broker.address.clone(),
                root: joined.broker.broker.identity().fingerprint(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity {
                id: joined.id,
                keys: identity,
            },
            tcp_ports([echo.port]),
            Arc::clone(&joined.sequence) as Arc<dyn Sequence>,
            short_probes(),
        ));
        let replacing = tokio::spawn({
            let second = Arc::clone(&second);

            async move {
                let _ = second.serve(std::future::pending()).await;
            }
        });

        // The first connection is terminated by its replacement (STM-11), which is observable
        // as its serve call returning.
        wait_until(|| joined.serving.is_finished()).await;

        // The release is the replaced connection's, and the broker completes it on its own
        // schedule rather than when the agent's serve returns. Asked once at that instant, a broker
        // that wrongly removed the registration is not caught over TCP — it has not got there yet —
        // so this watches across a window instead of sampling one moment of it.
        let watch_until = Instant::now() + AFTER_THE_RELEASE;
        while Instant::now() < watch_until {
            assert!(
                joined.connected_agents().tunnel_to(joined.id).is_some(),
                "CNF-6: releasing the replaced registration must not remove the replacement"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let mut stream = joined
            .connected_agents()
            .open(joined.id, &Selector::tcp(echo.port).unwrap())
            .await
            .expect("CNF-6: the agent must be reachable over the replacement");
        stream.write_all(b"still-here").await.unwrap();
        let mut echoed = [0u8; 10];
        stream.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"still-here");

        replacing.abort();
    }

    /// **CNF-7.** §6.2: a connection carrying no stream data for longer than (M+1)*K remains
    /// registered, both roles having answered probes.
    async fn cnf7_idle_connection_survives(&self) {
        let echo = Echo::tcp().await;
        let joined = self.joined(tcp_ports([echo.port])).await;

        tokio::time::sleep(short_probes().detection_bound() * 3).await;

        assert!(
            joined.connected_agents().tunnel_to(joined.id).is_some(),
            "CNF-7: an idle connection whose probes are answered must remain registered"
        );
        // "both roles having answered probes": the assertion above is the broker's half. An agent
        // whose own probes went unanswered would have ended the connection from its side, and its
        // serve task would have finished.
        assert!(
            !joined.serving.is_finished(),
            "CNF-7: the agent must have had its probes answered too, or it would have ended"
        );

        let mut stream = joined
            .connected_agents()
            .open(joined.id, &Selector::tcp(echo.port).unwrap())
            .await
            .expect("CNF-7: an idle connection must still carry streams");
        stream.write_all(b"alive").await.unwrap();
        let mut echoed = [0u8; 5];
        stream.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"alive");
    }

    /// **CNF-8.** §6.2: a peer that has ceased answering is deregistered within the detection
    /// bound while the broker continues to transmit to it. The relay drops everything after
    /// the cut without closing either side, which is what a host losing power looks like.
    async fn cnf8_silent_agent_dropped(&self) {
        let echo = Echo::tcp().await;
        let broker = self.broker().await;
        let relay = Relay::between(self.mapping, &broker.address.to_string()).await;

        let id = AgentId::new(uuid::Uuid::now_v7());
        let identity = Identity::generate().unwrap();
        broker.records.joined(id, identity.public_key());
        let agent = Arc::new(Agent::new(
            vec![self.mapping.agent_transport()],
            PinnedBroker {
                address: relay.address.clone(),
                root: broker.broker.identity().fingerprint(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity { id, keys: identity },
            tcp_ports([echo.port]),
            Counting::new(),
            short_probes(),
        ));
        let serving = tokio::spawn({
            let agent = Arc::clone(&agent);

            async move {
                let _ = agent.serve(std::future::pending()).await;
            }
        });
        wait_until(|| broker.connected_agents().tunnel_to(id).is_some()).await;

        let cut_at = Instant::now();
        relay.cut();

        wait_until(|| broker.connected_agents().tunnel_to(id).is_none()).await;
        let noticed = cut_at.elapsed();
        serving.abort();

        // §6.2 bounds detection at (M+1)*K from the last answered probe, and the cut lands
        // somewhere inside a probe interval, so the wait is that bound plus at most one more.
        // `settle` alone would accept anything under five seconds, which is not the bound §6.2
        // states and would pass an implementation that took whole seconds to notice.
        //
        // The half over is for a loaded machine, and it is what this can discriminate: detection
        // that has grown by half or more. A tolerance off by one lands at 750 ms against a correct
        // 600 ms and passes. Tightening far enough to catch that would leave the margin thinner
        // than the spread a busy runner shows, and a conformance member that flakes is worth less
        // than one that catches the order-of-magnitude case.
        let bound = short_probes().detection_bound() + short_probes().probe_interval();
        assert!(
            noticed <= bound * 3 / 2,
            "CNF-8: §6.2 bounds how long a peer that stopped answering stays registered; \
             deregistered after {noticed:?}, past {:?}",
            bound * 3 / 2
        );
    }

    /// **CNF-9.** VER-2, VER-4: a version octet outside the broker's range is answered
    /// `ACCESS`(Denied), the connection then ends, the version reaches the operator, and the
    /// policy is never consulted.
    ///
    /// The delay asserted here is TRN-10's, which binds every `ACCESS`(Denied) and names the
    /// version octet among the things it answers — CNF-12 covers the three denials that answer a
    /// `CLAIM`, and this covers the one that answers a version.
    ///
    /// What it cannot show is *where* the delay is timed from. Both mappings open the control
    /// stream lazily, so a broker reaches its first read only when the version octet arrives, and
    /// a clock started on the connection is indistinguishable from one started at receipt. TRN-10
    /// asks for receipt; a mapping that opened the stream eagerly would tell the difference.
    async fn cnf9_unsupported_version_denied(&self) {
        let reported = Reported::watching();
        let broker = self.broker().await;
        let identity = Identity::generate().unwrap();
        let mut connection = self.raw_dial(&broker, &identity).await;

        // A version this broker does not speak, distinct per mapping: the operator's line carries
        // the octet, and `said` is process-wide, so without that the mapping running alongside
        // this one would satisfy the assertion below.
        let unsupported = match self.mapping {
            Mapping::Tcp => 0x63,
            Mapping::Quic => 0x64,
        };
        assert_ne!(unsupported, wire::VERSION);

        let sent = Instant::now();
        connection.control.write_all(&[unsupported]).await.unwrap();
        connection.control.flush().await.unwrap();

        match read_frame_within(&mut connection, TIMEOUT).await {
            Some(Frame::Access(Access::Denied)) => assert!(
                sent.elapsed() >= DENIAL_DELAY,
                "CNF-9: TRN-10's delay binds the denial answering a version octet too"
            ),
            other => panic!("CNF-9: an unsupported version must be denied, got {other:?}"),
        }
        assert!(
            read_frame_within(&mut connection, TIMEOUT).await.is_none(),
            "CNF-9: the connection must end after a version denial"
        );
        assert_eq!(
            broker.records.consulted(),
            0,
            "CNF-9: an unsupported version must not reach the policy"
        );
        assert!(
            reported.said().iter().any(|said| {
                said.contains("unsupported protocol version")
                    && said.contains(&format!("agent_speaks={unsupported}"))
            }),
            "CNF-9: the denied version must reach the operator (VER-4), got {:?}",
            reported.said()
        );

        // VER-4 also asks that it be *distinguished* from a malformed frame, which the assertion
        // above cannot show on its own: a broker reporting both the same way would satisfy it. A
        // malformed frame reaches the operator as nothing resembling a version denial. Matched on
        // the frame-type octet, 0x7F, which neither mapping uses as its unsupported version, so a
        // process-wide capture cannot answer this from the other mapping's run.
        let malformed = Reported::watching();
        let mut connection = self.raw_dial(&broker, &identity).await;
        wire::write_version(&mut connection.control).await.unwrap();
        connection
            .control
            .write_all(&[0x7F, 0x00, 0x00])
            .await
            .unwrap();
        connection.control.flush().await.unwrap();
        assert!(
            read_frame_within(&mut connection, TIMEOUT).await.is_none(),
            "CNF-9: a malformed frame must end the connection unanswered"
        );

        let said = malformed.said();
        assert!(
            !said.iter().any(|said| {
                said.contains("unsupported protocol version") && said.contains("agent_speaks=127")
            }),
            "CNF-9: a malformed frame must not reach the operator as a version denial, got {said:?}"
        );
    }

    /// **CNF-10.** STM-3, STM-10: a malformed frame, and a second `CLAIM`, each end the
    /// connection, advance no state and modify no record.
    async fn cnf10_malformed_frame_ends_connection(&self) {
        let broker = self.broker().await;
        let identity = Identity::generate().unwrap();

        let probing = AgentId::new(uuid::Uuid::now_v7());
        let mut zero_sequence = vec![0x01, 0x00, 24];
        zero_sequence.extend_from_slice(probing.uuid().as_bytes());
        zero_sequence.extend_from_slice(&0u64.to_be_bytes());

        // The five §11 names that are frames rather than sequences of them. Each is rejected as it
        // is read, so each is sent on a connection of its own that has done nothing but declare a
        // version: nothing about the state it arrives in is what makes it malformed.
        let malformed = [
            ("an unknown frame type", vec![0x7F, 0x00, 0x00]),
            (
                "a CLAIM of the wrong length",
                vec![0x01, 0x00, 0x02, 0xAA, 0xBB],
            ),
            ("a sequence of 0", zero_sequence),
            (
                "a port of 0",
                vec![0x04, 0x00, 0x06, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00],
            ),
            (
                "a duplicate selector",
                vec![
                    0x04, 0x00, 0x09, 0x01, 0x00, 0x02, 0x01, 0x00, 0x16, 0x01, 0x00, 0x16,
                ],
            ),
        ];

        for (what, frame) in malformed {
            let mut connection = self.raw_dial(&broker, &identity).await;
            wire::write_version(&mut connection.control).await.unwrap();
            connection.control.write_all(&frame).await.unwrap();
            connection.control.flush().await.unwrap();

            assert!(
                read_frame_within(&mut connection, TIMEOUT).await.is_none(),
                "CNF-10: {what} must end the connection unanswered"
            );
            assert_eq!(
                broker.records.consulted(),
                0,
                "CNF-10: {what} must not reach the policy"
            );
            assert_eq!(
                broker.records.bound(probing),
                None,
                "CNF-10: {what} must modify no record"
            );
        }

        let id = AgentId::new(uuid::Uuid::now_v7());
        let mut connection = self.raw_dial(&broker, &identity).await;
        wire::write_version(&mut connection.control).await.unwrap();
        write_claim(&mut connection, id, 1).await;
        match read_frame_within(&mut connection, TIMEOUT).await {
            Some(Frame::Access(Access::Denied)) => {}
            other => panic!("CNF-10: an unjoined claim is denied, got {other:?}"),
        }
        write_claim(&mut connection, id, 2).await;
        assert!(
            read_frame_within(&mut connection, TIMEOUT).await.is_none(),
            "CNF-10: a second CLAIM must end the connection (STM-10)"
        );
        assert_eq!(
            broker.records.bound(id),
            None,
            "CNF-10: nothing may have been recorded for the probing identifier"
        );
    }

    /// **CNF-11.** POL-5, POL-2: following withdrawal the withdrawn keypair is denied, a token
    /// issued after withdrawal binds a new keypair, and a token presented for an identifier
    /// with a keypair already bound binds nothing.
    async fn cnf11_withdrawal_restores_join(&self) {
        let broker = self.broker().await;

        let id = AgentId::new(uuid::Uuid::now_v7());
        let old = Identity::generate().unwrap();
        broker.records.joined(id, old.public_key());

        // A token for an identifier that already has a key binds nothing (POL-2).
        broker.records.joined(id, old.public_key());
        let intruder = Identity::generate().unwrap();
        let agent = Agent::new(
            vec![self.mapping.agent_transport()],
            PinnedBroker {
                address: broker.address.clone(),
                root: broker.broker.identity().fingerprint(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity { id, keys: intruder },
            tcp_ports([1]),
            Counting::new(),
            short_probes(),
        );
        assert!(
            agent.join(&SecretString::from("stolen")).await.is_err(),
            "CNF-11: a token cannot displace a bound keypair"
        );
        assert_eq!(broker.records.bound(id), Some(old.public_key()));

        broker.records.withdraw(id);
        broker.records.unbound(id, "fresh-token");

        let old_agent = Agent::new(
            vec![self.mapping.agent_transport()],
            PinnedBroker {
                address: broker.address.clone(),
                root: broker.broker.identity().fingerprint(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity {
                id,
                keys: Identity::from_pkcs8_der(old.private_key_der()).unwrap(),
            },
            tcp_ports([1]),
            Counting::new(),
            short_probes(),
        );
        assert!(
            matches!(
                old_agent.serve(std::future::pending()).await,
                Err(TunnelError::Refused)
            ),
            "CNF-11: the withdrawn keypair must be denied"
        );

        let renewed = Identity::generate().unwrap();
        let public = renewed.public_key();
        let agent = Agent::new(
            vec![self.mapping.agent_transport()],
            PinnedBroker {
                address: broker.address.clone(),
                root: broker.broker.identity().fingerprint(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity { id, keys: renewed },
            tcp_ports([1]),
            Counting::new(),
            short_probes(),
        );
        agent
            .join(&SecretString::from("fresh-token"))
            .await
            .expect("CNF-11: a token issued after withdrawal must bind a new keypair");
        assert_eq!(broker.records.bound(id), Some(public));
    }

    /// **CNF-12.** SEC-7: an unjoined identifier, an identifier unknown to the broker, and a
    /// keypair other than the bound one produce answers indistinguishable in content and
    /// realisation, each under the delay of TRN-10.
    async fn cnf12_denial_is_uniform(&self) {
        // A denial leaves the connection open for a JOIN (STM-10), so this is how long to watch for
        // anything further rather than how long to wait for it.
        const AFTER_THE_ANSWER: Duration = Duration::from_millis(200);

        let broker = self.broker().await;
        let identity = Identity::generate().unwrap();

        let unknown = AgentId::new(uuid::Uuid::now_v7());

        let known_unbound = AgentId::new(uuid::Uuid::now_v7());
        broker.records.unbound(known_unbound, "waiting");

        let wrong_key = AgentId::new(uuid::Uuid::now_v7());
        broker
            .records
            .joined(wrong_key, Identity::generate().unwrap().public_key());

        let mut realisations = Vec::new();

        for (id, case) in [
            (unknown, "an unknown identifier"),
            (known_unbound, "a known identifier with nothing bound"),
            (wrong_key, "a keypair other than the bound one"),
        ] {
            let mut connection = self.raw_dial(&broker, &identity).await;
            wire::write_version(&mut connection.control).await.unwrap();

            let asked = Instant::now();
            write_claim(&mut connection, id, 1).await;

            let answer = read_frame_within(&mut connection, TIMEOUT).await;
            match answer {
                Some(Frame::Access(Access::Denied)) => {}
                ref other => panic!("CNF-12: {case} must be denied, got {other:?}"),
            }

            let waited = asked.elapsed();
            assert!(
                waited >= DENIAL_DELAY,
                "CNF-12: {case} must be answered no earlier than the delay of TRN-10, \
                 answered after {waited:?}"
            );

            // SEC-7 asks for indistinguishable *realisation* as well as content, and what the
            // connection does after the answer is as observable as the answer. A broker that ended
            // one case's connection and left another's open would separate them here.
            let then = read_frame_within(&mut connection, AFTER_THE_ANSWER).await;
            assert!(
                then.is_none(),
                "CNF-12: nothing further may follow the denial of {case}, got {then:?}"
            );

            realisations.push((case, format!("{answer:?}"), format!("{then:?}")));
        }

        let (first, answer, then) = &realisations[0];
        for (case, other_answer, other_then) in &realisations[1..] {
            assert_eq!(
                other_answer, answer,
                "CNF-12: {case} must be answered exactly as {first} is"
            );
            assert_eq!(
                other_then, then,
                "CNF-12: the connection after {case} must behave as it does after {first}"
            );
        }
    }

    /// **CNF-13.** POL-6: a claim whose sequence does not exceed the last accepted is denied
    /// and freezes the identifier, including against the keypair and sequence that were
    /// current a moment before; a claim skipping sequences is admitted.
    async fn cnf13_stale_sequence_freezes(&self) {
        let reported = Reported::watching();
        let broker = self.broker().await;
        let identity = Identity::generate().unwrap();
        let id = AgentId::new(uuid::Uuid::now_v7());
        broker.records.joined(id, identity.public_key());

        // Skipping sequences is ordinary (STM-15): 5, then 50.
        assert_eq!(
            self.claim_outcome(&broker, &identity, id, 5).await,
            Access::Granted,
            "CNF-13: a first claim is admitted whatever its sequence"
        );
        assert_eq!(
            self.claim_outcome(&broker, &identity, id, 50).await,
            Access::Granted,
            "CNF-13: a claim skipping sequences must be admitted"
        );

        // The same sequence again: one keypair in two places.
        assert_eq!(
            self.claim_outcome(&broker, &identity, id, 50).await,
            Access::Denied,
            "CNF-13: a sequence that does not exceed the last accepted must be denied"
        );
        assert!(
            broker.records.bound(id).is_none(),
            "CNF-13: freezing is POL-5 applied to the identifier, so no keypair stays bound"
        );

        assert_eq!(
            self.claim_outcome(&broker, &identity, id, 51).await,
            Access::Denied,
            "CNF-13: a frozen identifier must deny the keypair that was current a moment before"
        );

        // STM-16, which no wire answer can carry: a freeze is denied indistinguishably from
        // every other denial (SEC-7), so the operator report is the whole of what SEC-6 detects.
        let said = reported.said();
        assert!(
            said.iter()
                .any(|said| said.contains("agent blocked") && said.contains(&id.to_string())),
            "CNF-13: a freeze must reach the operator with the agent it froze, got {said:?}"
        );
    }

    /// **CNF-22.** Freezing is POL-5 applied to the identifier, so recovery is a fresh token
    /// and a further join. That only works because binding clears the recorded sequence: a
    /// holder that froze an identifier at `u64::MAX` leaves nothing an honest agent could
    /// exceed, and without the clearing the identifier would be lost rather than frozen.
    async fn cnf22_joining_recovers_a_frozen_identifier(&self) {
        let broker = self.broker().await;

        // "recovered no differently from any other": the same recovery at the top of the range and
        // at an ordinary number. Freezing only at the top would be passed by an implementation that
        // clears the recorded sequence for `u64::MAX` alone.
        for froze_at in [u64::MAX, 50] {
            let frozen_by = Identity::generate().unwrap();
            let id = AgentId::new(uuid::Uuid::now_v7());
            broker.records.joined(id, frozen_by.public_key());

            // A sequence, then itself again: nothing below it could ever exceed it, so an
            // identifier left recorded at that number is unrecoverable without the clearing.
            assert_eq!(
                self.claim_outcome(&broker, &frozen_by, id, froze_at).await,
                Access::Granted
            );
            assert_eq!(
                self.claim_outcome(&broker, &frozen_by, id, froze_at).await,
                Access::Denied
            );
            assert!(
                broker.records.bound(id).is_none(),
                "CNF-22: freezing unbinds the keypair, froze at {froze_at}"
            );

            // An operator invites it back, and a replacement agent joins with a keypair of its own.
            broker.records.unbound(id, "fresh-token");
            let keys = Identity::generate().unwrap();
            let claiming = Identity::from_pkcs8_der(keys.private_key_der()).unwrap();
            let agent = Agent::new(
                vec![self.mapping.agent_transport()],
                PinnedBroker {
                    address: broker.address.clone(),
                    root: broker.broker.identity().fingerprint(),
                    rollback: Arc::new(InMemoryRollbackIndex::default()),
                },
                AgentIdentity { id, keys },
                tcp_ports([1]),
                Counting::new(),
                short_probes(),
            );
            agent
                .join(&SecretString::from("fresh-token"))
                .await
                .expect("CNF-22: a token issued after a freeze binds a new keypair");
            assert_eq!(broker.records.bound(id), Some(claiming.public_key()));

            // A fresh agent counts from 1, which is below the sequence that froze the identifier.
            assert_eq!(
                self.claim_outcome(&broker, &claiming, id, 1).await,
                Access::Granted,
                "CNF-22: binding clears the recorded sequence, froze at {froze_at}"
            );
        }
    }

    /// **CNF-14.** POL-1 before POL-6: a low sequence with a keypair other than the bound one
    /// is denied and freezes nothing; the identifier is still reachable afterwards by its own
    /// agent.
    async fn cnf14_sequence_needs_key_first(&self) {
        let broker = self.broker().await;
        let identity = Identity::generate().unwrap();
        let id = AgentId::new(uuid::Uuid::now_v7());
        broker.records.joined(id, identity.public_key());

        assert_eq!(
            self.claim_outcome(&broker, &identity, id, 10).await,
            Access::Granted
        );

        let prober = Identity::generate().unwrap();
        assert_eq!(
            self.claim_outcome(&broker, &prober, id, 1).await,
            Access::Denied,
            "CNF-14: a keypair other than the bound one is denied"
        );
        assert!(
            broker.records.bound(id).is_some(),
            "CNF-14: a denied keypair must freeze nothing"
        );

        assert_eq!(
            self.claim_outcome(&broker, &identity, id, 11).await,
            Access::Granted,
            "CNF-14: the identifier must still be reachable by its own agent"
        );
    }

    /// **CNF-15.** STM-11: an identifier that admits a second connection has the first ended,
    /// and a third has the second ended; a connection admitted for another identifier is
    /// unaffected.
    async fn cnf15_admission_displaces_connection(&self) {
        let echo = Echo::tcp().await;
        let joined = self.joined(tcp_ports([echo.port])).await;
        let broker = &joined.broker;
        let identity = Identity::from_pkcs8_der(&joined.key_der).unwrap();

        // A second admission under the same identifier ends the serving connection (STM-11).
        let mut second = self.raw_dial(broker, &identity).await;
        wire::write_version(&mut second.control).await.unwrap();
        write_claim(&mut second, joined.id, 1_000).await;
        assert!(matches!(
            read_frame_within(&mut second, TIMEOUT).await,
            Some(Frame::Access(Access::Granted))
        ));
        wait_until(|| joined.serving.is_finished()).await;

        // A third ends the second.
        let mut third = self.raw_dial(broker, &identity).await;
        wire::write_version(&mut third.control).await.unwrap();
        write_claim(&mut third, joined.id, 1_001).await;
        assert!(matches!(
            read_frame_within(&mut third, TIMEOUT).await,
            Some(Frame::Access(Access::Granted))
        ));
        assert!(
            read_frame_within(&mut second, TIMEOUT).await.is_none(),
            "CNF-15: a further admission under one identifier must end the connection before it"
        );

        // A connection admitted for another identifier is unaffected.
        let other_id = AgentId::new(uuid::Uuid::now_v7());
        let other_identity = Identity::generate().unwrap();
        broker.records.joined(other_id, other_identity.public_key());
        let mut other = self.raw_dial(broker, &other_identity).await;
        wire::write_version(&mut other.control).await.unwrap();
        write_claim(&mut other, other_id, 1).await;
        assert!(matches!(
            read_frame_within(&mut other, TIMEOUT).await,
            Some(Frame::Access(Access::Granted))
        ));
        assert!(
            broker.connected_agents().tunnel_to(joined.id).is_some(),
            "CNF-15: an admission for another identifier must not displace this one"
        );
    }

    /// **CNF-16.** TRN-6: a stream held open and relaying does not prevent a second stream
    /// from being opened and relaying on the same connection.
    async fn cnf16_concurrent_streams(&self) {
        let (echo, also_echo) = (Echo::tcp().await, Echo::tcp().await);
        let joined = self.joined(tcp_ports([echo.port, also_echo.port])).await;

        let mut held = joined
            .connected_agents()
            .open(joined.id, &Selector::tcp(echo.port).unwrap())
            .await
            .expect("CNF-16: the first stream must open");
        held.write_all(b"held").await.unwrap();
        let mut echoed = [0u8; 4];
        held.read_exact(&mut echoed).await.unwrap();

        // The first stream is left open and unfinished on purpose: what must not block the
        // second is a relay in progress.
        let mut second = tokio::time::timeout(
            TIMEOUT,
            joined
                .connected_agents()
                .open(joined.id, &Selector::tcp(also_echo.port).unwrap()),
        )
        .await
        .expect("CNF-16: a stream held open must not stop a second stream being opened")
        .expect("CNF-16: the second stream must open");

        second.write_all(b"beside").await.unwrap();
        let mut alongside = [0u8; 6];
        second.read_exact(&mut alongside).await.unwrap();
        assert_eq!(&alongside, b"beside");

        held.write_all(b"more").await.unwrap();
        held.read_exact(&mut echoed).await.unwrap();
        assert_eq!(
            &echoed, b"more",
            "CNF-16: the first stream must still relay after the second has"
        );
    }

    /// **CNF-17.** §2.5: a `udp-port` stream relays datagrams in both directions with
    /// boundaries preserved, an empty datagram included, and never splits or merges one.
    async fn cnf17_datagram_boundaries_preserved(&self) {
        let echo = Echo::udp().await;
        let selector = Selector::udp(echo.port).unwrap();
        let joined = self
            .joined(Arc::new(Selectors::new([selector.clone().into()])))
            .await;

        let stream = joined
            .connected_agents()
            .open(joined.id, &selector)
            .await
            .expect("CNF-17: the offered udp-port must be reachable");
        let (mut from_agent, mut to_agent) = tokio::io::split(stream);

        // Two records back to back: boundaries must survive, not byte counts.
        wire::write_record(&mut to_agent, b"first").await.unwrap();
        wire::write_record(&mut to_agent, b"second-longer")
            .await
            .unwrap();

        let mut first = Vec::new();
        wire::read_record(&mut from_agent, &mut first)
            .await
            .unwrap();
        let mut second = Vec::new();
        wire::read_record(&mut from_agent, &mut second)
            .await
            .unwrap();
        assert_eq!(first, b"first", "CNF-17: a datagram must come back whole");
        assert_eq!(
            second, b"second-longer",
            "CNF-17: datagrams must not be split or merged"
        );

        wire::write_record(&mut to_agent, b"").await.unwrap();
        let mut empty = Vec::new();
        wire::read_record(&mut from_agent, &mut empty)
            .await
            .unwrap();
        assert!(
            empty.is_empty(),
            "CNF-17: an empty datagram is a datagram and must round-trip"
        );
    }

    /// **CNF-18.** §2.3: a `unix-path` stream reaches the UNIX-domain socket POL-8 returns for
    /// it, and data returns over it.
    #[cfg_attr(not(unix), allow(clippy::unused_async))]
    async fn cnf18_socket_path_reaches_service(&self) {
        #[cfg(unix)]
        {
            let echo = Echo::unix();
            let selector = Selector::unix(&echo.path).unwrap();
            let joined = self
                .joined(Arc::new(Selectors::new([selector.clone().into()])))
                .await;

            let mut stream = joined
                .connected_agents()
                .open(joined.id, &selector)
                .await
                .expect("CNF-18: the offered unix-path must be reachable");

            stream.write_all(b"over-a-socket").await.unwrap();
            let mut echoed = [0u8; 13];
            stream.read_exact(&mut echoed).await.unwrap();
            assert_eq!(&echoed, b"over-a-socket");
        }
    }

    /// **CNF-19.** STM-14: an agent whose claim is denied during serving establishes no
    /// further connection within 5 seconds of the connection's end, and a further denial
    /// lengthens the delay.
    ///
    /// Observed as the spacing of POL-1 consultations. Each denial costs the broker's fixed
    /// delay (TRN-10) before the connection ends, so the spacing floor is that delay plus the
    /// backoff; without STM-14 the spacings would sit at the delay alone.
    async fn cnf19_denied_agent_delays_redial(&self) {
        // STM-14 draws the two delays from [5 s, 10 s) and [10 s, 20 s), past `TIMEOUT`.
        const ATTEMPT_WAIT: Duration = Duration::from_secs(30);

        let broker = self.broker().await;

        let id = AgentId::new(uuid::Uuid::now_v7());
        let agent = Arc::new(Agent::new(
            vec![self.mapping.agent_transport()],
            PinnedBroker {
                address: broker.address.clone(),
                root: broker.broker.identity().fingerprint(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity {
                id,
                keys: Identity::generate().unwrap(),
            },
            tcp_ports([1]),
            Counting::new(),
            short_probes(),
        ));

        let dialling = tokio::spawn({
            let agent = Arc::clone(&agent);

            async move {
                agent.serve_continuously(std::future::pending()).await;
            }
        });

        let attempt = |n: usize| {
            let records = Arc::clone(&broker.records);

            async move {
                tokio::time::timeout(ATTEMPT_WAIT, async {
                    while records.consulted() < n {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("CNF-19: the agent must dial again after its delay");

                Instant::now()
            }
        };

        let first = attempt(1).await;
        let second = attempt(2).await;
        let third = attempt(3).await;
        dialling.abort();

        let floor =
            |backoff: Duration| (DENIAL_DELAY + backoff).saturating_sub(Duration::from_millis(50));
        assert!(
            second - first >= floor(Duration::from_secs(5)),
            "CNF-19: the second attempt must wait at least 5 seconds beyond the denial, waited {:?}",
            second - first
        );
        assert!(
            third - second >= floor(Duration::from_secs(10)),
            "CNF-19: a further denial must lengthen the delay (B doubles), waited {:?}",
            third - second
        );
    }

    /// **CNF-20.** §3.1, STM-13: an `ANNOUNCE` is answered by nothing and decides nothing.
    ///
    /// The agent announces one selector and serves another, which a broker that took the
    /// announced set for permission would refuse, and a broker that answered the frame would
    /// end the connection over.
    async fn cnf20_announcement_decides_nothing(&self) {
        // How long to watch for a reply that must never come, not how long to wait for one.
        const ANSWERED_BY_NOTHING: Duration = Duration::from_millis(200);

        let served = Echo::tcp().await;
        let announced_only = Echo::tcp().await;
        let reported = Reported::watching();

        // Announces one port, serves a different one: the two disagree on purpose.
        let target = Arc::new(Disagreeing {
            announced: Selector::tcp(announced_only.port).unwrap(),
            served: Selector::tcp(served.port).unwrap(),
        });
        let joined = self.joined(target).await;

        // The connection is still serving after the announcement it sent on reaching SERVING.
        wait_until({
            let broker = Arc::clone(&joined.broker.broker);
            let id = joined.id;

            move || broker.connected_agents().tunnel_to(id).is_some()
        })
        .await;

        let mut stream = joined
            .connected_agents()
            .open(joined.id, &Selector::tcp(served.port).unwrap())
            .await
            .expect("CNF-20: a selector POL-8 serves is reachable though it was never announced");
        stream.write_all(b"unannounced").await.unwrap();
        let mut echoed = [0u8; 11];
        stream.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"unannounced");

        let refused = joined
            .connected_agents()
            .open(joined.id, &Selector::tcp(announced_only.port).unwrap())
            .await;
        assert!(
            matches!(refused, Err(TunnelError::Refused)),
            "CNF-20: an announced selector POL-8 has no target for must still be refused"
        );

        // A second announcement, and the connection carries on regardless.
        joined
            .agent
            .reconfigure(Arc::new(Disagreeing {
                announced: Selector::tcp(served.port).unwrap(),
                served: Selector::tcp(served.port).unwrap(),
            }))
            .expect("a set of one selector fits one frame");
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            joined.connected_agents().tunnel_to(joined.id).is_some(),
            "CNF-20: an announcement must not end the connection that sent it"
        );
        assert!(
            joined
                .connected_agents()
                .open(joined.id, &Selector::tcp(served.port).unwrap())
                .await
                .is_ok(),
            "CNF-20: the tunnel still serves after a second announcement"
        );

        // STM-13, which CNF-20 names and no other member reaches: deriving nothing from an
        // announcement is half of it, and reporting it to the operator is the other half. Without
        // this the report could be deleted and every member of §11 would still pass.
        let said = reported.said();
        assert!(
            said.iter().any(|said| {
                said.contains("announced the services it exposes")
                    && said.contains(&joined.id.to_string())
            }),
            "CNF-20: an announced set must reach the operator with the agent it came from, got \
             {said:?}"
        );

        // "answered by no message". Everything above drives a real agent, which never sees whether
        // a reply arrived — a broker that answered an ANNOUNCE would leave every assertion so far
        // standing. A raw connection at SERVING can watch for one. Its own identifier, so admitting
        // it does not displace the connection the assertions above rely on (STM-11).
        let watching = AgentId::new(uuid::Uuid::now_v7());
        let identity = Identity::generate().unwrap();
        joined
            .broker
            .records
            .joined(watching, identity.public_key());

        let mut connection = self.raw_dial(&joined.broker, &identity).await;
        wire::write_version(&mut connection.control).await.unwrap();
        write_claim(&mut connection, watching, 1).await;
        assert!(
            matches!(
                read_frame_within(&mut connection, TIMEOUT).await,
                Some(Frame::Access(Access::Granted))
            ),
            "CNF-20: a bound keypair reaches SERVING"
        );

        wire::write_frame(
            &mut connection.control,
            &Frame::Announce(crate::AnnounceSet::new([Selector::tcp(served.port)
                .unwrap()
                .into()])),
        )
        .await
        .unwrap();
        connection.control.flush().await.unwrap();

        let answer = read_frame_within(&mut connection, ANSWERED_BY_NOTHING).await;
        assert!(
            answer.is_none(),
            "CNF-20: an ANNOUNCE must be answered by no message, got {answer:?}"
        );
    }

    /// **CNF-21.** TRN-6, §6.1: past the mapping's per-connection stream ceiling an open is
    /// refused, and the connection survives to serve the streams it holds.
    ///
    /// A consumer asking for more forwards than a connection can carry must not cost an agent
    /// its tunnel: the ceiling is a refusal, not a fault of the session.
    async fn cnf21_forwards_past_the_ceiling(&self) {
        let echo = Echo::tcp().await;
        let joined = self.joined(tcp_ports([echo.port])).await;
        let selector = Selector::tcp(echo.port).unwrap();

        // Hold streams until one is refused. The ceiling belongs to the mapping, so the count
        // is whatever it turns out to be; that an open is answered at all is the point.
        let mut held = Vec::new();
        let refused = loop {
            match tokio::time::timeout(
                TIMEOUT,
                joined.connected_agents().open(joined.id, &selector),
            )
            .await
            {
                Err(elapsed) => panic!(
                    "CNF-21: an open past the ceiling must be answered, not left waiting \
                     ({elapsed}, TRN-6); {} were held",
                    held.len()
                ),
                Ok(Ok(stream)) => held.push(stream),
                Ok(Err(refused)) => break refused,
            }

            assert!(
                held.len() < 4096,
                "CNF-21: this mapping bounds nothing, so the ceiling cannot be reached"
            );
        };

        assert!(
            matches!(refused, TunnelError::Refused),
            "CNF-21: the ceiling is a refusal, not an absence: the agent is still connected"
        );
        assert!(
            joined.connected_agents().tunnel_to(joined.id).is_some(),
            "CNF-21: reaching the ceiling must not end the agent's tunnel"
        );

        // The streams already open still relay.
        let mut relaying = held.pop().expect("the ceiling is above one stream");
        relaying.write_all(b"still-here").await.unwrap();
        let mut echoed = [0u8; 10];
        relaying.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"still-here");

        // And room comes back when they close. Closing is not instant on either mapping — the
        // session reaps a stream when it next runs — so this waits rather than asking once.
        drop(relaying);
        drop(held);

        let deadline = Instant::now() + TIMEOUT;
        loop {
            if joined
                .connected_agents()
                .open(joined.id, &selector)
                .await
                .is_ok()
            {
                break;
            }

            assert!(
                Instant::now() < deadline,
                "CNF-21: closing the held streams must let a further open succeed"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// **CNF-23.** §2.3 and §2.5: a selector that names no service announces a set and opens no
    /// stream. An agent announcing one range serves every port the range identifies.
    ///
    /// The other half of CNF-23 — that a preamble naming no service is closed rather than
    /// answered — is structural here rather than behavioural, so it is not asserted below. A
    /// preamble carries a [`Selector`], which has no set-naming variant, so no broker built on
    /// this crate can encode one and no test written against this API can ask for one. That the
    /// agent refuses the octets if another implementation sends them is covered by
    /// `wire::tests::read_stream_preamble_of_selector_naming_no_service_refuses`.
    async fn cnf23_announced_set_is_not_dialable(&self) {
        let echo = Echo::tcp().await;
        let joined = self
            .joined(Arc::new(Selectors::new([Announced::TcpRange(
                PortRange::every(),
            )])))
            .await;

        let mut stream = joined
            .connected_agents()
            .open(joined.id, &Selector::tcp(echo.port).unwrap())
            .await
            .expect("CNF-23: a port the announced range identifies must be reachable");

        stream.write_all(b"through-the-range").await.unwrap();
        let mut echoed = [0u8; 17];
        stream.read_exact(&mut echoed).await.unwrap();

        assert_eq!(
            &echoed, b"through-the-range",
            "CNF-23: bytes must round-trip on a port the announced range identifies"
        );
    }
}

#[cfg(test)]
mod tests {
    /// One test per requirement, over both mappings, so a failure names the requirement that broke
    /// and the mapping it broke on — and so libtest runs them at once. §11 asks for a per-member
    /// verdict; a single test that stops at the first failure cannot give one.
    macro_rules! conformance {
        ($($requirement:ident,)+) => {
            $(
                mod $requirement {
                    use crate::conformance::Suite;

                    #[tokio::test(flavor = "multi_thread")]
                    async fn over_tcp() {
                        Suite::tcp().$requirement().await;
                    }

                    #[tokio::test(flavor = "multi_thread")]
                    async fn over_quic() {
                        Suite::quic().$requirement().await;
                    }
                }
            )+
        };
    }

    conformance! {
        cnf1_reaches_offered_services,
        cnf2_refuses_selector_never_offered,
        cnf3_join_binds_proven_keypair,
        cnf4_denies_unjoined_or_wrong_keyed_claim,
        cnf5_agent_refuses_unpinned_broker,
        cnf6_replacement_wins,
        cnf7_idle_connection_survives,
        cnf8_silent_agent_dropped,
        cnf9_unsupported_version_denied,
        cnf10_malformed_frame_ends_connection,
        cnf11_withdrawal_restores_join,
        cnf12_denial_is_uniform,
        cnf13_stale_sequence_freezes,
        cnf14_sequence_needs_key_first,
        cnf15_admission_displaces_connection,
        cnf16_concurrent_streams,
        cnf17_datagram_boundaries_preserved,
        cnf18_socket_path_reaches_service,
        cnf19_denied_agent_delays_redial,
        cnf20_announcement_decides_nothing,
        cnf21_forwards_past_the_ceiling,
        cnf22_joining_recovers_a_frozen_identifier,
        cnf23_announced_set_is_not_dialable,
    }
}
