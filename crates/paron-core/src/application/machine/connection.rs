use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use async_trait::async_trait;
use futures_util::{SinkExt as _, StreamExt as _, sink};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::commands::open_terminal::OpenTerminalCommand;
use super::commands::test_connection::{self, TestConnectionCommand};
use super::errors::{OpenTerminalError, TestConnectionError};
use super::event as machine;
use super::ports::{
    MachineTransport, SshKeypairs, Target, TerminalEvent, TerminalExit, TerminalInput,
    TerminalSession, TransportError,
};
use super::service::MachineService;
use super::use_cases::MachineConnections;
use crate::application::audit::{AuditService, Entity};
use crate::application::events::{Event, EventService, EventSubscription, SubscriptionScope};
use crate::application::identity::authorization::actor::Actor;
use crate::application::identity::errors::TouchSessionError;
use crate::application::identity::event as user;
use crate::application::identity::use_cases::Sessions;
use crate::application::repository::TxContext;
use crate::application::tasks::Tasks;
use crate::domain::machine::{ConnectionTest, ProtocolKind, Provider, Terminal};
use crate::domain::user::{Permission, Role};

pub struct MachineConnectionService<Tx> {
    machines: Arc<MachineService<Tx>>,
    transport: Arc<dyn MachineTransport>,
    keypairs: Arc<dyn SshKeypairs>,
    audit: Arc<AuditService<Tx>>,
    events: Arc<EventService>,
    sessions: Arc<dyn Sessions>,
    tasks: Tasks,
    idle_timeout: Duration,
}

impl<Tx: Send + Sync + 'static> MachineConnectionService<Tx> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        machines: Arc<MachineService<Tx>>,
        transport: Arc<dyn MachineTransport>,
        keypairs: Arc<dyn SshKeypairs>,
        audit: Arc<AuditService<Tx>>,
        events: Arc<EventService>,
        sessions: Arc<dyn Sessions>,
        tasks: Tasks,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            machines,
            transport,
            keypairs,
            audit,
            events,
            sessions,
            tasks,
            idle_timeout,
        }
    }

    pub async fn test(
        &self,
        actor: &Actor,
        cmd: TestConnectionCommand,
    ) -> Result<(), TestConnectionError> {
        actor.authorize(Permission::MachineOperate)?;

        let machine = match cmd.machine {
            test_connection::Machine::Stored(id) => {
                self.machines.get(&mut TxContext::none(), actor, id).await?
            }
            test_connection::Machine::Changed(update) => {
                let stored = self
                    .machines
                    .get(&mut TxContext::none(), actor, update.id)
                    .await?;

                update.apply_to(&stored, &*self.keypairs)?
            }
            test_connection::Machine::New(create) => create.into_domain(&*self.keypairs),
        };

        Ok(self
            .transport
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: machine
                    .capability::<ConnectionTest>(Provider::Protocol(cmd.protocol))?,
            })
            .await?)
    }

    pub async fn open_terminal(
        &self,
        actor: Actor,
        cmd: OpenTerminalCommand,
    ) -> Result<TerminalSession, OpenTerminalError> {
        actor.authorize(Permission::MachineOperate)?;

        let user_event_stream = match actor.user() {
            Some(authenticated) => Some(
                self.events
                    .subscribe::<user::EventKind>(SubscriptionScope::One(authenticated.user.id))
                    .await
                    .context("could not subscribe to user events")?,
            ),
            // System, can't lose access.
            None => None,
        };

        let machine_event_stream = self
            .events
            .subscribe::<machine::EventKind>(SubscriptionScope::One(cmd.machine_id))
            .await
            .context("could not subscribe to machine events")?;

        let machine = self
            .machines
            .get(&mut TxContext::none(), &actor, cmd.machine_id)
            .await?;

        let terminal = self
            .transport
            .terminal(
                Target {
                    machine_id: machine.id,
                    connectivity: machine.connectivity.clone(),
                    capability: machine.capability::<Terminal>(Provider::Protocol(cmd.protocol))?,
                },
                cmd.size,
                cmd.terminal,
            )
            .await?;

        let terminal_session_id = uuid::Uuid::now_v7();
        let machine_entity = Entity::from(&machine);

        self.audit
            .append(
                &mut TxContext::none(),
                &actor,
                machine_entity.clone(),
                machine::AuditKind::TerminalOpened {
                    session_id: terminal_session_id,
                    protocol: cmd.protocol,
                },
            )
            .await?;

        let (last_activity_tx, last_activity_rx) = watch::channel(tokio::time::Instant::now());
        let (typed_tx, typed_rx) = watch::channel(());
        let closed = CancellationToken::new();

        let actor = Arc::new(actor);
        let work = {
            let actor = Arc::clone(&actor);
            let sessions = Arc::clone(&self.sessions);
            let idle_timeout = self.idle_timeout;
            let closed = closed.clone();

            async move {
                tokio::select! {
                    () = watchdog(
                        &actor,
                        &*sessions,
                        user_event_stream,
                        machine_event_stream,
                        cmd.protocol,
                        last_activity_rx,
                        typed_rx,
                        idle_timeout,
                    ) => {}
                    () = closed.cancelled() => {}
                }
            }
        };
        let cleanup = {
            let audit = Arc::clone(&self.audit);
            let closed = closed.clone();

            async move {
                closed.cancel();

                audit
                    .append(
                        &mut TxContext::none(),
                        &*actor,
                        machine_entity,
                        machine::AuditKind::TerminalClosed {
                            session_id: terminal_session_id,
                        },
                    )
                    .await
                    .with_context(|| {
                        format!("could not append terminal closed audit entry for terminal session {terminal_session_id}")
                    })
            }
        };
        self.tasks.spawn(work, cleanup);

        Ok(supervised(terminal, closed, last_activity_tx, typed_tx))
    }

    pub async fn handle_machine_event(&self, event: &Event<machine::EventKind>) {
        if let machine::EventKind::Deleted
        | machine::EventKind::Updated {
            enrollment_ended: true,
            ..
        } = &event.kind
        {
            self.transport.disconnect(event.entity_id).await;
        }
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> MachineConnections for MachineConnectionService<Tx> {
    async fn test(
        &self,
        actor: &Actor,
        cmd: TestConnectionCommand,
    ) -> Result<(), TestConnectionError> {
        Self::test(self, actor, cmd).await
    }

    async fn open_terminal(
        &self,
        actor: Actor,
        cmd: OpenTerminalCommand,
    ) -> Result<TerminalSession, OpenTerminalError> {
        Self::open_terminal(self, actor, cmd).await
    }
}

/// The same terminal, which tells the watchdog of its activity and ends when the watchdog says so.
fn supervised(
    terminal: TerminalSession,
    closed: CancellationToken,
    last_activity_tx: watch::Sender<tokio::time::Instant>,
    typed_tx: watch::Sender<()>,
) -> TerminalSession {
    let close_guard = Arc::new(closed.clone().drop_guard());

    let output = {
        let mut output = terminal.output;
        let closed = closed.clone();
        let close_guard = Arc::clone(&close_guard);
        let last_activity_tx = last_activity_tx.clone();

        async_stream::stream! {
            let _guard = close_guard;

            loop {
                let event = tokio::select! {
                    biased;
                    () = closed.cancelled() => TerminalEvent::Exit(TerminalExit::Disconnected),
                    event = output.next() => {
                        event.unwrap_or(TerminalEvent::Exit(TerminalExit::Disconnected))
                    }
                };

                last_activity_tx.send_replace(tokio::time::Instant::now());
                let is_exit = matches!(event, TerminalEvent::Exit(_));
                yield event;
                if is_exit {
                    break;
                }
            }
        }
    };
    let input = sink::unfold(
        (
            terminal.input,
            closed,
            close_guard,
            last_activity_tx,
            typed_tx,
        ),
        |(mut input, closed, close_guard, last_activity_tx, typed_tx), message: TerminalInput| async move {
            if closed.is_cancelled() {
                return Err(TransportError::Unreachable("the terminal is closed".into()));
            }

            last_activity_tx.send_replace(tokio::time::Instant::now());
            if matches!(message, TerminalInput::Data(_)) {
                typed_tx.send_replace(());
            }
            input.send(message).await?;

            Ok((input, closed, close_guard, last_activity_tx, typed_tx))
        },
    );

    TerminalSession {
        input: Box::pin(input),
        output: Box::pin(output),
    }
}

#[allow(clippy::too_many_arguments)]
async fn watchdog(
    actor: &Actor,
    sessions: &dyn Sessions,
    mut user_event_stream: Option<EventSubscription<user::EventKind>>,
    mut machine_event_stream: EventSubscription<machine::EventKind>,
    protocol: ProtocolKind,
    last_activity_rx: watch::Receiver<tokio::time::Instant>,
    mut typed_rx: watch::Receiver<()>,
    idle_timeout: Duration,
) {
    const SESSION_TOUCH_PERIOD: Duration = Duration::from_mins(1);

    let mut next_session_touch = tokio::time::Instant::now();

    loop {
        tokio::select! {
            typed = async {
                tokio::time::sleep_until(next_session_touch).await;

                typed_rx.changed().await
            } => {
                if typed.is_err() {
                    return;
                }

                match sessions.touch_session(actor).await {
                    Ok(()) => {}
                    Err(TouchSessionError::SessionEnded) => return,
                    Err(TouchSessionError::Unexpected(error)) => {
                        tracing::warn!("could not touch session: {error:#}");
                    }
                }

                next_session_touch = tokio::time::Instant::now() + SESSION_TOUCH_PERIOD;
            }
            event = async {
                match &mut user_event_stream {
                    Some(user_event_stream) => user_event_stream.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let event = match event {
                    Ok(event) => event.kind,
                    Err(error) => {
                        tracing::warn!("could not read user events, closing the terminal: {error}");

                        return;
                    }
                };
                let Some(authenticated) = actor.user() else {
                    continue;
                };

                match event {
                    user::EventKind::SessionsDeleted { session_ids }
                        if session_ids.contains(&authenticated.session_id) =>
                    {
                        return;
                    }
                    user::EventKind::RolesChanged { roles }
                        if roles.is_disjoint(&Role::all_granting(Permission::MachineOperate)) =>
                    {
                        return;
                    }
                    _ => {}
                }
            }
            event = machine_event_stream.next() => {
                match event {
                    Ok(event) => match event.kind {
                        machine::EventKind::Deleted
                        | machine::EventKind::Updated {
                            connectivity_changed: true,
                            ..
                        } => return,
                        machine::EventKind::Updated {
                            protocols_changed, ..
                        } if protocols_changed.contains(&protocol) => return,
                        machine::EventKind::Updated { .. } => {}
                    },
                    Err(error) => {
                        tracing::warn!("could not read machine events, closing the terminal: {error}");

                        return;
                    }
                }
            }
            () = tokio::time::sleep_until(*last_activity_rx.borrow() + idle_timeout) => {
                if last_activity_rx.borrow().elapsed() >= idle_timeout {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::collections::BTreeSet;
    use std::num::NonZeroU16;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::FutureExt as _;
    use secrecy::SecretString;

    use super::*;
    use crate::application::audit::{self, MockRepository as MockAuditRepository};
    use crate::application::identity::use_cases::MockSessions;
    use crate::application::machine::commands::create::{
        AuthenticationCommand, ConnectivityCommand, CreateMachineCommand, ProtocolsCommand,
        SshCommand,
    };
    use crate::application::machine::commands::update::{
        UpdateMachineCommand, UpdateProtocolsCommand,
    };
    use crate::application::machine::event::{AuditKind as MachineAuditKind, EventKind};
    use crate::application::machine::ports::{
        MockMachineTransport, MockRepository, MockSshKeypairs, TerminalSize, TerminalType,
    };
    use crate::application::repository::RepositoryError;
    use crate::domain::machine::{
        CapabilityUnavailable, ConnectionTest, Connectivity, Host, Machine, MachineName,
        ProtocolKind, ssh,
    };
    use crate::infrastructure::in_memory::events::InMemoryEventStream;
    use crate::testing::clock::clock_at;
    use crate::testing::logs::CapturedLogs;
    use crate::testing::mocks::transactions_that_commit;
    use crate::testing::{events, identity, keys, machine};

    const WAIT_LIMIT: Duration = Duration::from_secs(5);

    fn now() -> jiff::Timestamp {
        jiff::Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_hours(1)
    }

    struct ServiceBuilder {
        repository: MockRepository<()>,
        transport: MockMachineTransport,
        audit: Arc<dyn audit::Repository<()>>,
        sessions: MockSessions,
        events: Arc<InMemoryEventStream>,
        tasks: Tasks,
    }

    impl ServiceBuilder {
        /// The repository holds `stored`, and the terminal accepts all input and gives no output.
        fn new(stored: &Machine) -> Self {
            let mut repository = MockRepository::new();
            repository.expect_get_by_id().returning({
                let stored = stored.clone();
                move |_, _| Ok(stored.clone())
            });
            let mut transport = MockMachineTransport::new();
            transport.expect_terminal().returning(|_, _, _| {
                Ok(TerminalSession {
                    input: Box::pin(sink::drain().sink_map_err(|never| match never {})),
                    output: Box::pin(futures_util::stream::pending()),
                })
            });

            let mut sessions = MockSessions::new();
            sessions.expect_touch_session().returning(|_| Ok(()));

            Self {
                repository,
                transport,
                audit: events::recorded_audit(),
                sessions,
                events: Arc::new(InMemoryEventStream::default()),
                tasks: Tasks::default(),
            }
        }

        fn repository(mut self, repository: MockRepository<()>) -> Self {
            self.repository = repository;
            self
        }

        fn transport(mut self, transport: MockMachineTransport) -> Self {
            self.transport = transport;
            self
        }

        fn audit(mut self, audit: Arc<dyn audit::Repository<()>>) -> Self {
            self.audit = audit;
            self
        }

        fn sessions(mut self, sessions: MockSessions) -> Self {
            self.sessions = sessions;
            self
        }

        fn events(mut self, events: Arc<InMemoryEventStream>) -> Self {
            self.events = events;
            self
        }

        fn tasks(mut self, tasks: Tasks) -> Self {
            self.tasks = tasks;
            self
        }

        fn build(self) -> MachineConnectionService<()> {
            let transactions = Arc::new(transactions_that_commit());
            let audit = Arc::new(AuditService::new(self.audit, clock_at(now())));
            let events = events::event_service_on(self.events, clock_at(now()));

            MachineConnectionService::new(
                Arc::new(MachineService::new(
                    Arc::new(self.repository),
                    Arc::clone(&transactions) as Arc<_>,
                    Arc::new(MockSshKeypairs::new()),
                    Arc::clone(&audit),
                    Arc::clone(&events),
                )),
                Arc::new(self.transport),
                Arc::new(MockSshKeypairs::new()),
                Arc::clone(&audit),
                Arc::clone(&events),
                Arc::new(self.sessions),
                self.tasks,
                identity::session_policy().max_idle_timeout,
            )
        }
    }

    fn open_command(machine: &Machine) -> OpenTerminalCommand {
        OpenTerminalCommand {
            machine_id: machine.id,
            protocol: ProtocolKind::Ssh,
            size: TerminalSize {
                columns: NonZeroU16::new(80).unwrap(),
                rows: NonZeroU16::new(24).unwrap(),
            },
            terminal: TerminalType::XtermTrueColor,
        }
    }

    async fn next_output(terminal: &mut TerminalSession) -> Option<TerminalEvent> {
        tokio::time::timeout(WAIT_LIMIT, terminal.output.next())
            .await
            .expect("the terminal gives output in time")
    }

    #[tokio::test]
    async fn open_terminal_on_machine_without_ssh_returns_missing() {
        let stored = machine::machine_builder().without_ssh().build();
        let mut transport = MockMachineTransport::new();
        transport.expect_terminal().never();
        let service = ServiceBuilder::new(&stored).transport(transport).build();

        let opened = service
            .open_terminal(Actor::System, open_command(&stored))
            .await;

        assert_matches!(
            opened.err(),
            Some(OpenTerminalError::CapabilityUnavailable(
                CapabilityUnavailable::Missing(ProtocolKind::Ssh)
            ))
        );
    }

    #[tokio::test]
    async fn open_terminal_then_drop_records_the_open_and_the_close_of_the_same_terminal() {
        let stored = machine::direct_machine();
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new(&stored)
            .audit(Arc::clone(&audited) as Arc<_>)
            .build();
        let terminal = service
            .open_terminal(Actor::System, open_command(&stored))
            .await
            .unwrap();

        drop(terminal);

        let entries = tokio::time::timeout(WAIT_LIMIT, audited.wait_for_recorded(2))
            .await
            .expect("the close of the terminal is recorded in time");
        let audit::AuditKind::Machine(MachineAuditKind::TerminalOpened {
            session_id,
            protocol: ProtocolKind::Ssh,
        }) = entries[0].kind
        else {
            panic!("the first entry is the open of the terminal over SSH");
        };
        assert_eq!(
            entries[1..],
            [audit::AuditEntry {
                kind: MachineAuditKind::TerminalClosed { session_id }.into(),
                ..entries[1].clone()
            }]
        );
    }

    #[tokio::test]
    async fn open_terminal_then_stop_the_tasks_records_the_close_and_ends_the_terminal() {
        let stored = machine::direct_machine();
        let audited = events::recorded_audit();
        let tasks = Tasks::default();
        let service = ServiceBuilder::new(&stored)
            .audit(Arc::clone(&audited) as Arc<_>)
            .tasks(tasks.clone())
            .build();
        let mut terminal = service
            .open_terminal(Actor::System, open_command(&stored))
            .await
            .unwrap();

        tasks.stop().await;

        let audit::AuditKind::Machine(MachineAuditKind::TerminalOpened { session_id, .. }) =
            audited.recorded()[0].kind
        else {
            panic!("the first entry is the open of the terminal");
        };
        assert_eq!(
            audited.recorded_kinds()[1..],
            [MachineAuditKind::TerminalClosed { session_id }.into()]
        );
        assert_eq!(
            next_output(&mut terminal).await,
            Some(TerminalEvent::Exit(TerminalExit::Disconnected))
        );
    }

    #[tokio::test]
    async fn open_terminal_then_drop_with_a_failing_audit_log_logs_the_error() {
        let logs = CapturedLogs::capture();
        let stored = machine::direct_machine();
        let appended = Arc::new(AtomicUsize::new(0));
        let mut failing_after_the_open = MockAuditRepository::new();
        failing_after_the_open
            .expect_create()
            .returning(move |_, _| {
                if appended.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(())
                } else {
                    Err(RepositoryError::Unexpected(anyhow::anyhow!(
                        "the audit log is full"
                    )))
                }
            });
        let terminal = ServiceBuilder::new(&stored)
            .audit(Arc::new(failing_after_the_open))
            .build()
            .open_terminal(Actor::System, open_command(&stored))
            .await
            .unwrap();

        drop(terminal);

        let line = tokio::time::timeout(
            WAIT_LIMIT,
            logs.wait_for_line(|line| {
                line.contains("could not append terminal closed audit entry")
            }),
        )
        .await
        .expect("the failed close of the terminal is logged in time");
        assert!(line.contains("the audit log is full"), "{line}");
    }

    #[tokio::test]
    async fn open_terminal_then_the_session_of_the_actor_is_deleted_disconnects() {
        let stored = machine::direct_machine();
        let actor = identity::actor_with_roles(&[Role::Operator]);
        let authenticated = actor.user().unwrap().clone();
        let stream = Arc::new(InMemoryEventStream::default());
        let mut terminal = ServiceBuilder::new(&stored)
            .events(Arc::clone(&stream))
            .build()
            .open_terminal(actor, open_command(&stored))
            .await
            .unwrap();

        events::event_service_on(stream, clock_at(now())).publish(
            authenticated.user.id,
            user::EventKind::SessionsDeleted {
                session_ids: vec![authenticated.session_id],
            },
        );

        assert_eq!(
            next_output(&mut terminal).await,
            Some(TerminalEvent::Exit(TerminalExit::Disconnected))
        );
    }

    #[tokio::test]
    async fn open_terminal_then_input_after_the_session_is_deleted_disconnects() {
        let stored = machine::direct_machine();
        let mut sessions = MockSessions::new();
        sessions
            .expect_touch_session()
            .returning(|_| Err(TouchSessionError::SessionEnded));
        let mut terminal = ServiceBuilder::new(&stored)
            .sessions(sessions)
            .build()
            .open_terminal(
                identity::actor_with_roles(&[Role::Operator]),
                open_command(&stored),
            )
            .await
            .unwrap();

        terminal
            .input
            .send(TerminalInput::Data(b"ls\n".to_vec()))
            .await
            .unwrap();

        assert_eq!(
            next_output(&mut terminal).await,
            Some(TerminalEvent::Exit(TerminalExit::Disconnected))
        );
    }

    #[tokio::test]
    async fn open_terminal_then_input_touches_the_session_of_the_user() {
        let stored = machine::direct_machine();
        let actor = identity::actor_with_roles(&[Role::Operator]);
        let session_id = actor.user().unwrap().session_id;
        let (touched_tx, mut touched_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sessions = MockSessions::new();
        sessions.expect_touch_session().returning(move |actor| {
            touched_tx
                .send(actor.user().map(|authenticated| authenticated.session_id))
                .unwrap();
            Ok(())
        });
        let service = ServiceBuilder::new(&stored).sessions(sessions).build();
        let mut terminal = service
            .open_terminal(actor, open_command(&stored))
            .await
            .unwrap();

        terminal
            .input
            .send(TerminalInput::Data(b"ls\n".to_vec()))
            .await
            .unwrap();

        let touched = tokio::time::timeout(WAIT_LIMIT, touched_rx.recv())
            .await
            .expect("the session is touched in time");
        assert_eq!(touched, Some(Some(session_id)));
    }

    #[tokio::test(start_paused = true)]
    async fn open_terminal_with_no_input_and_no_output_for_the_idle_limit_disconnects() {
        let stored = machine::direct_machine();
        let idle_timeout = identity::session_policy().max_idle_timeout;
        let service = ServiceBuilder::new(&stored).build();
        let mut terminal = service
            .open_terminal(Actor::System, open_command(&stored))
            .await
            .unwrap();

        tokio::time::advance(idle_timeout / 2).await;
        let before_the_limit = terminal.output.next().now_or_never();
        tokio::time::advance(idle_timeout / 2).await;
        let at_the_limit = terminal.output.next().await;

        assert_eq!(before_the_limit, None);
        assert_eq!(
            at_the_limit,
            Some(TerminalEvent::Exit(TerminalExit::Disconnected))
        );
    }

    #[tokio::test]
    async fn open_terminal_then_a_change_of_connectivity_disconnects() {
        let stored = machine::direct_machine();
        let stream = Arc::new(InMemoryEventStream::default());
        let mut terminal = ServiceBuilder::new(&stored)
            .events(Arc::clone(&stream))
            .build()
            .open_terminal(Actor::System, open_command(&stored))
            .await
            .unwrap();

        events::event_service_on(stream, clock_at(now())).publish(
            stored.id,
            EventKind::Updated {
                connectivity_changed: true,
                protocols_changed: BTreeSet::new(),
                enrollment_ended: false,
            },
        );

        assert_eq!(
            next_output(&mut terminal).await,
            Some(TerminalEvent::Exit(TerminalExit::Disconnected))
        );
    }

    #[tokio::test]
    async fn open_terminal_then_the_machine_is_deleted_disconnects() {
        let stored = machine::direct_machine();
        let stream = Arc::new(InMemoryEventStream::default());
        let mut terminal = ServiceBuilder::new(&stored)
            .events(Arc::clone(&stream))
            .build()
            .open_terminal(Actor::System, open_command(&stored))
            .await
            .unwrap();

        events::event_service_on(stream, clock_at(now())).publish(stored.id, EventKind::Deleted);

        assert_eq!(
            next_output(&mut terminal).await,
            Some(TerminalEvent::Exit(TerminalExit::Disconnected))
        );
    }

    #[tokio::test]
    async fn test_without_machine_operate_returns_denied() {
        let stored = machine::direct_machine();
        let mut transport = MockMachineTransport::new();
        transport.expect_verify().never();
        let service = ServiceBuilder::new(&stored).transport(transport).build();

        let tested = service
            .test(
                &identity::actor_with_roles(&[]),
                TestConnectionCommand {
                    protocol: ProtocolKind::Ssh,
                    machine: test_connection::Machine::Stored(stored.id),
                },
            )
            .await;

        assert_matches!(tested, Err(TestConnectionError::Denied(_)));
    }

    #[tokio::test]
    async fn test_new_machine_verifies_it_without_saving_it() {
        let stored = machine::direct_machine();
        let mut repository = MockRepository::new();
        repository.expect_create().never();
        let mut transport = MockMachineTransport::new();
        transport
            .expect_verify()
            .once()
            .withf(|target| {
                matches!(&target.capability, ConnectionTest::Ssh(access) if &*access.user == "build")
            })
            .returning(|_| Ok(()));
        let service = ServiceBuilder::new(&stored)
            .repository(repository)
            .transport(transport)
            .build();

        service
            .test(
                &Actor::System,
                TestConnectionCommand {
                    protocol: ProtocolKind::Ssh,
                    machine: test_connection::Machine::New(CreateMachineCommand {
                        name: MachineName::parse("build").unwrap(),
                        connectivity: ConnectivityCommand::Direct {
                            host: Host::parse(machine::DIRECT_HOST).unwrap(),
                        },
                        protocols: ProtocolsCommand {
                            ssh: Some(SshCommand {
                                port: machine::SSH_PORT,
                                user: ssh::User::parse("build").unwrap(),
                                host_key: keys::ssh_public_key(),
                                authentication: AuthenticationCommand::Password(
                                    ssh::Password::parse(SecretString::from(machine::SSH_PASSWORD))
                                        .unwrap(),
                                ),
                                connect_timeout: ssh::ConnectTimeout::new(machine::CONNECT_TIMEOUT)
                                    .unwrap(),
                            }),
                        },
                    }),
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_with_mismatched_host_key_returns_host_key_mismatch() {
        let stored = machine::direct_machine();
        let mut transport = MockMachineTransport::new();
        transport
            .expect_verify()
            .returning(|_| Err(TransportError::HostKeyMismatch));
        let service = ServiceBuilder::new(&stored).transport(transport).build();

        let tested = service
            .test(
                &Actor::System,
                TestConnectionCommand {
                    protocol: ProtocolKind::Ssh,
                    machine: test_connection::Machine::Stored(stored.id),
                },
            )
            .await;

        assert_matches!(
            tested,
            Err(TestConnectionError::Transport(
                TransportError::HostKeyMismatch
            ))
        );
    }

    #[tokio::test]
    async fn test_changed_machine_verifies_the_changes_without_saving_them() {
        let stored = machine::direct_machine();
        let mut repository = MockRepository::new();
        repository.expect_get_by_id().returning({
            let stored = stored.clone();
            move |_, _| Ok(stored.clone())
        });
        repository.expect_update().never();
        let mut transport = MockMachineTransport::new();
        transport
            .expect_verify()
            .once()
            .withf(|target| {
                matches!(&target.connectivity, Connectivity::Direct { host } if host.to_string() == "changed.example")
            })
            .returning(|_| Ok(()));
        let service = ServiceBuilder::new(&stored)
            .repository(repository)
            .transport(transport)
            .build();

        service
            .test(
                &Actor::System,
                TestConnectionCommand {
                    protocol: ProtocolKind::Ssh,
                    machine: test_connection::Machine::Changed(UpdateMachineCommand {
                        id: stored.id,
                        name: None,
                        connectivity: Some(ConnectivityCommand::Direct {
                            host: Host::parse("changed.example").unwrap(),
                        }),
                        protocols: UpdateProtocolsCommand::default(),
                    }),
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_with_a_protocol_the_machine_lacks_returns_missing() {
        let stored = machine::machine_builder().without_ssh().build();
        let mut transport = MockMachineTransport::new();
        transport.expect_verify().never();
        let service = ServiceBuilder::new(&stored).transport(transport).build();

        let tested = service
            .test(
                &Actor::System,
                TestConnectionCommand {
                    protocol: ProtocolKind::Ssh,
                    machine: test_connection::Machine::Stored(stored.id),
                },
            )
            .await;

        assert_matches!(
            tested,
            Err(TestConnectionError::CapabilityUnavailable(
                CapabilityUnavailable::Missing(ProtocolKind::Ssh)
            ))
        );
    }

    fn machine_event(kind: EventKind) -> Event<EventKind> {
        Event {
            id: uuid::Uuid::now_v7(),
            created_at: now(),
            entity_id: uuid::Uuid::now_v7(),
            kind,
        }
    }

    #[tokio::test]
    async fn handle_machine_event_deleted_disconnects_the_agent() {
        let event = machine_event(EventKind::Deleted);
        let agent = event.entity_id;
        let mut transport = MockMachineTransport::new();
        transport
            .expect_disconnect()
            .once()
            .withf(move |machine_id| *machine_id == agent)
            .returning(|_| ());
        let service = ServiceBuilder::new(&machine::enrolled_machine())
            .transport(transport)
            .build();

        service.handle_machine_event(&event).await;
    }

    #[tokio::test]
    async fn handle_machine_event_end_of_the_enrollment_disconnects_the_agent() {
        let event = machine_event(EventKind::Updated {
            connectivity_changed: true,
            protocols_changed: BTreeSet::new(),
            enrollment_ended: true,
        });
        let mut transport = MockMachineTransport::new();
        transport.expect_disconnect().once().returning(|_| ());
        let service = ServiceBuilder::new(&machine::enrolled_machine())
            .transport(transport)
            .build();

        service.handle_machine_event(&event).await;
    }

    #[tokio::test]
    async fn handle_machine_event_change_of_protocol_keeps_the_agent() {
        let event = machine_event(EventKind::Updated {
            connectivity_changed: false,
            protocols_changed: BTreeSet::from([ProtocolKind::Ssh]),
            enrollment_ended: false,
        });
        let mut transport = MockMachineTransport::new();
        transport.expect_disconnect().never();
        let service = ServiceBuilder::new(&machine::enrolled_machine())
            .transport(transport)
            .build();

        service.handle_machine_event(&event).await;
    }
}
