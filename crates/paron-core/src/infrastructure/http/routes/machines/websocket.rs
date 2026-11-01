use std::num::NonZeroU16;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use futures_util::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use tracing::Instrument as _;
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::{IntoParams, OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::application::identity::authorization::actor::Actor;
use crate::application::machine::{
    OpenTerminalCommand, OpenTerminalError, TerminalEvent, TerminalExit, TerminalInput,
    TerminalSession, TerminalSize, TerminalType,
};
use crate::infrastructure::http::auth::CurrentUser;
use crate::infrastructure::http::error::{self, ApiError};
use crate::infrastructure::http::state::Deps;
use serde_utoipa_validate::{newtype, validate};

/// The largest message that Paron accepts from the browser. The browser sends input in pieces of
/// 16 KiB, and its other messages are a few bytes. Paron closes the connection if a message is larger.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// The largest quantity of output that the browser has not acknowledged. At this limit, Paron stops
/// reading from the terminal. The SSH window then fills, and the program on the machine waits. Thus
/// Paron holds no more than one SSH window of output, and Ctrl+C has an effect quickly.
const MAX_UNACKNOWLEDGED_BYTES: usize = 256 * 1024;

/// No operation refers to these messages, because they go through the WebSocket. This struct adds
/// them to the OpenAPI document. The browser then gets its types from the same source as the server.
#[derive(OpenApi)]
#[openapi(components(schemas(TerminalClientMessage, TerminalServerMessage)))]
struct TerminalMessages;

pub fn routes() -> OpenApiRouter<Deps> {
    OpenApiRouter::with_openapi(TerminalMessages::openapi()).routes(routes!(open_terminal))
}

newtype! {
    TerminalDimension: u64 -> NonZeroU16 {
        schema: ObjectBuilder::new()
            .schema_type(Type::Integer)
            .minimum(Some(1))
            .maximum(Some(u16::MAX)),
        deserialize: |raw: u64| {
            u16::try_from(raw)
                .ok()
                .and_then(NonZeroU16::new)
                .ok_or("expected an integer between 1 and 65535")
        },
        serialize: |dimension: &NonZeroU16| Ok(dimension.get()),
    }
}

newtype! {
    TerminalTypeName: String -> TerminalType {
        XtermTrueColor => "xterm-truecolor",
    }
}

#[validate]
#[derive(Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
struct TerminalQuery {
    cols: TerminalDimension,
    rows: TerminalDimension,
    terminal: TerminalTypeName,
    protocol: super::Protocol,
}

#[validate]
#[derive(Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalClientMessage {
    Resize {
        cols: TerminalDimension,
        rows: TerminalDimension,
    },
    Ack {
        bytes: u64,
    },
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalServerMessage {
    Opened,
    ExitStatus { status: u32 },
    ExitSignal { signal: String },
    Error { error: ApiError },
}

impl From<TerminalExit> for TerminalServerMessage {
    fn from(exit: TerminalExit) -> Self {
        match exit {
            TerminalExit::Status(status) => Self::ExitStatus { status },
            TerminalExit::Signal(signal) => Self::ExitSignal { signal },
            TerminalExit::Disconnected => Self::Error {
                error: ApiError::detail(
                    StatusCode::BAD_GATEWAY,
                    "the connection to the machine was lost",
                ),
            },
        }
    }
}

impl From<TerminalServerMessage> for Message {
    #[expect(
        clippy::expect_used,
        reason = "the messages are plain data, which serde_json serializes without failure"
    )]
    fn from(message: TerminalServerMessage) -> Self {
        let text = serde_json::to_string(&message).expect("could not serialize to JSON");

        Self::Text(text.into())
    }
}

#[utoipa::path(
    get,
    path = "/machines/{id}/terminal",
    tag = "machines",
    operation_id = "open_machine_terminal",
    params(TerminalQuery),
    responses((status = SWITCHING_PROTOCOLS)),
    security(("session" = [])),
)]
async fn open_terminal(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
    Query(query): Query<TerminalQuery>,
    ws: WebSocketUpgrade,
) -> error::Result<Response> {
    let span = tracing::info_span!("terminal", user = %user.user.id);
    let actor = Actor::from(user);
    let query = query.validated()?;
    let cmd = OpenTerminalCommand {
        machine_id: id,
        protocol: query.protocol,
        size: TerminalSize {
            columns: query.cols,
            rows: query.rows,
        },
        terminal: query.terminal,
    };

    Ok(ws
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| {
            serve_terminal_websocket(socket, deps, actor, cmd).instrument(span)
        }))
}

async fn serve_terminal_websocket(
    mut ws: WebSocket,
    deps: Deps,
    actor: Actor,
    cmd: OpenTerminalCommand,
) {
    let start_time = std::time::Instant::now();
    let last_message = match deps.machine_connections.open_terminal(actor, cmd).await {
        Ok(terminal) => {
            tracing::info!("terminal opened");
            stream_terminal(&mut ws, terminal, deps.config.websocket_keepalive_interval).await
        }
        Err(error) => {
            tracing::info!("could not open terminal: {error}");
            Some(TerminalServerMessage::Error {
                error: error.into(),
            })
        }
    };

    tracing::info!(
        duration_seconds = start_time.elapsed().as_secs(),
        "terminal closed: {reason}",
        reason = last_message.as_ref().map_or_else(
            || "client disconnected".into(),
            |message| format!("{message:?}")
        )
    );

    let Some(last_message) = last_message else {
        return;
    };
    let code = match &last_message {
        TerminalServerMessage::Error { error } => match error.status() {
            StatusCode::PAYLOAD_TOO_LARGE => close_code::SIZE,
            status if status.is_client_error() => close_code::POLICY,
            _ => close_code::ERROR,
        },
        _ => close_code::NORMAL,
    };
    let _ = ws.send(last_message.into()).await;
    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: "".into(),
        })))
        .await;
}

async fn stream_terminal(
    ws: &mut WebSocket,
    mut terminal: TerminalSession,
    keepalive_interval: std::time::Duration,
) -> Option<TerminalServerMessage> {
    let mut ping = tokio::time::interval_at(
        tokio::time::Instant::now() + keepalive_interval,
        keepalive_interval,
    );
    let mut unacknowledged_bytes = 0;

    ws.send(TerminalServerMessage::Opened.into()).await.ok()?;

    loop {
        tokio::select! {
            event = terminal.output.next(), if unacknowledged_bytes < MAX_UNACKNOWLEDGED_BYTES => match event {
                Some(TerminalEvent::Output(bytes)) => {
                    unacknowledged_bytes += bytes.len();
                    ws.send(Message::Binary(bytes.into())).await.ok()?;
                }
                Some(TerminalEvent::Exit(exit)) => return Some(exit.into()),
                None => return Some(TerminalExit::Disconnected.into()),
            },
            message = ws.recv() => match message? {
                Ok(frame) => {
                    let result = match frame {
                        Message::Binary(bytes) => {
                            terminal.input.send(TerminalInput::Data(bytes.into())).await
                        }
                        Message::Text(text) => {
                            let message = serde_json::from_str::<TerminalClientMessage>(&text)
                                .map_err(|error| ApiError::detail(StatusCode::BAD_REQUEST, error.to_string()))
                                .and_then(|message| Ok(message.validated()?));
                            match message {
                                Ok(TerminalClientMessageParsed::Resize { cols, rows }) => {
                                    terminal.input.send(TerminalInput::Resize(TerminalSize { columns: cols, rows })).await
                                }
                                Ok(TerminalClientMessageParsed::Ack { bytes }) => {
                                    unacknowledged_bytes = unacknowledged_bytes.saturating_sub(usize::try_from(bytes).unwrap_or(usize::MAX));
                                    Ok(())
                                }
                                Err(error) => return Some(TerminalServerMessage::Error { error }),
                            }
                        }
                        Message::Ping(_) | Message::Pong(_) => Ok(()),
                        Message::Close(_) => return None,
                    };
                    if result.is_err() {
                        return Some(TerminalExit::Disconnected.into());
                    }
                }
                Err(error) => {
                    let error = error.into_inner();
                    if let Some(tungstenite::Error::Capacity(_)) = error.downcast_ref() {
                        return Some(TerminalServerMessage::Error { error: ApiError::detail(StatusCode::PAYLOAD_TOO_LARGE, format!(
                            "the message is larger than {} KiB",
                            MAX_MESSAGE_BYTES / 1024
                        )) });
                    }
                    tracing::debug!("could not read from the WebSocket: {error}");
                    return None;
                }
            },
            _ = ping.tick() => ws.send(Message::Ping(axum::body::Bytes::new())).await.ok()?,
        }
    }
}

impl From<OpenTerminalError> for ApiError {
    fn from(error: OpenTerminalError) -> Self {
        match error {
            OpenTerminalError::Denied(denied) => denied.into(),
            OpenTerminalError::NotFound => Self::detail(StatusCode::NOT_FOUND, error.to_string()),
            OpenTerminalError::CapabilityUnavailable(_) => {
                Self::detail(StatusCode::BAD_REQUEST, error.to_string())
            }
            OpenTerminalError::Transport(error) => error.into(),
            OpenTerminalError::Unexpected(cause) => cause.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::num::NonZeroU16;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use axum::http::header;
    use serde_json::json;
    use tokio::sync::mpsc;
    use tokio_tungstenite::tungstenite::Message as Frame;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame as BrowserCloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::Frame as RawFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::{CloseCode, Data, OpCode};

    use super::*;
    use crate::application::identity::authorization::actor::Denied;
    use crate::application::machine::{
        MockMachineConnections, OpenTerminalError, TerminalType, TransportError,
    };
    use crate::domain::machine::ProtocolKind;
    use crate::domain::user::Permission;
    use crate::infrastructure::http::router;
    use crate::testing::http::{self, SESSION_COOKIE};

    type BrowserSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// An open terminal. The test writes the output of the terminal and reads its input.
    struct ConnectedTerminal {
        browser: BrowserSocket,
        terminal_output: mpsc::UnboundedSender<TerminalEvent>,
        terminal_input: mpsc::UnboundedReceiver<TerminalInput>,
        terminal_events_read: Arc<AtomicUsize>,
    }

    async fn connect_browser(
        machine_connections: MockMachineConnections,
        machine_id: uuid::Uuid,
    ) -> BrowserSocket {
        let api = router(Deps {
            machine_connections: Arc::new(machine_connections),
            ..http::deps()
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });

        let mut request = format!(
            "ws://{address}/api/machines/{machine_id}/terminal?cols=80&rows=24&terminal=xterm-truecolor&protocol=ssh"
        )
        .into_client_request()
        .unwrap();
        request
            .headers_mut()
            .insert(header::COOKIE, SESSION_COOKIE.parse().unwrap());
        request
            .headers_mut()
            .insert(header::HOST, http::HOST.parse().unwrap());

        tokio_tungstenite::connect_async(request).await.unwrap().0
    }

    async fn open_terminal_session() -> ConnectedTerminal {
        let (terminal_input_sender, terminal_input) = mpsc::unbounded_channel();
        let (terminal_output, terminal_events) = mpsc::unbounded_channel();
        let terminal_events_read = Arc::new(AtomicUsize::new(0));

        let input_sink = futures_util::sink::unfold(
            terminal_input_sender,
            async |terminal_input_sender, input| {
                let _ = terminal_input_sender.send(input);
                Ok::<_, TransportError>(terminal_input_sender)
            },
        );
        let output_stream = futures_util::stream::unfold(
            (terminal_events, Arc::clone(&terminal_events_read)),
            async |(mut terminal_events, terminal_events_read)| {
                let event = terminal_events.recv().await?;
                terminal_events_read.fetch_add(1, Ordering::SeqCst);
                Some((event, (terminal_events, terminal_events_read)))
            },
        );
        let terminal = TerminalSession {
            input: Box::pin(input_sink),
            output: Box::pin(output_stream),
        };
        let mut machine_connections = MockMachineConnections::new();
        machine_connections
            .expect_open_terminal()
            .return_once(move |_, _| Ok(terminal));

        let mut browser = connect_browser(machine_connections, uuid::Uuid::now_v7()).await;
        assert_eq!(
            text_frame_json(&next_frame(&mut browser).await),
            json!({ "type": "opened" })
        );

        ConnectedTerminal {
            browser,
            terminal_output,
            terminal_input,
            terminal_events_read,
        }
    }

    async fn next_frame(browser: &mut BrowserSocket) -> Frame {
        tokio::time::timeout(Duration::from_secs(5), browser.next())
            .await
            .expect("a frame within the time")
            .expect("the socket is still open")
            .expect("a well-formed frame")
    }

    async fn next_terminal_input(
        terminal_input: &mut mpsc::UnboundedReceiver<TerminalInput>,
    ) -> Option<TerminalInput> {
        tokio::time::timeout(Duration::from_secs(5), terminal_input.recv())
            .await
            .expect("the terminal receives something within the time")
    }

    fn text_frame_json(frame: &Frame) -> serde_json::Value {
        match frame {
            Frame::Text(text) => serde_json::from_str(text).unwrap(),
            unexpected_frame => panic!("expected a text frame, got {unexpected_frame:?}"),
        }
    }

    #[tokio::test]
    async fn serve_terminal_websocket_denied_for_the_session_user_sends_error_and_closes_with_1008()
    {
        let machine_id = uuid::Uuid::now_v7();
        let mut machine_connections = MockMachineConnections::new();
        machine_connections
            .expect_open_terminal()
            .once()
            .withf(move |actor, cmd| {
                http::is_signed_in_user(actor)
                    && *cmd
                        == OpenTerminalCommand {
                            machine_id,
                            protocol: ProtocolKind::Ssh,
                            size: TerminalSize {
                                columns: NonZeroU16::new(80).unwrap(),
                                rows: NonZeroU16::new(24).unwrap(),
                            },
                            terminal: TerminalType::XtermTrueColor,
                        }
            })
            .return_once(|_, _| {
                Err(OpenTerminalError::Denied(Denied::MissingPermission(
                    Permission::MachineOperate,
                )))
            });
        let mut browser = connect_browser(machine_connections, machine_id).await;

        let error_frame = next_frame(&mut browser).await;

        assert_eq!(text_frame_json(&error_frame)["error"]["status"], 403);
        assert_matches!(
            next_frame(&mut browser).await,
            Frame::Close(Some(BrowserCloseFrame {
                code: CloseCode::Policy,
                ..
            }))
        );
    }

    #[tokio::test]
    async fn serve_terminal_websocket_on_unreachable_machine_sends_error_and_closes_with_1011() {
        let mut machine_connections = MockMachineConnections::new();
        machine_connections
            .expect_open_terminal()
            .return_once(|_, _| {
                Err(OpenTerminalError::Transport(TransportError::Unreachable(
                    "no route to the machine".into(),
                )))
            });
        let mut browser = connect_browser(machine_connections, uuid::Uuid::now_v7()).await;

        let error_frame = next_frame(&mut browser).await;

        assert_eq!(text_frame_json(&error_frame)["error"]["status"], 502);
        assert_matches!(
            next_frame(&mut browser).await,
            Frame::Close(Some(BrowserCloseFrame {
                code: CloseCode::Error,
                ..
            }))
        );
    }

    #[tokio::test]
    async fn serve_terminal_websocket_output_arrives_as_binary_frames() {
        let mut session = open_terminal_session().await;

        session
            .terminal_output
            .send(TerminalEvent::Output(b"hello".to_vec()))
            .unwrap();

        assert_eq!(
            next_frame(&mut session.browser).await,
            Frame::Binary(b"hello".to_vec().into())
        );
    }

    #[tokio::test]
    async fn serve_terminal_websocket_binary_frame_reaches_the_terminal() {
        let mut session = open_terminal_session().await;

        session
            .browser
            .send(Frame::Binary(b"ls\r".to_vec().into()))
            .await
            .unwrap();

        assert_eq!(
            next_terminal_input(&mut session.terminal_input).await,
            Some(TerminalInput::Data(b"ls\r".to_vec()))
        );
    }

    #[tokio::test]
    async fn serve_terminal_websocket_resize_message_resizes_the_terminal() {
        let mut session = open_terminal_session().await;

        session
            .browser
            .send(Frame::text(r#"{"type":"resize","cols":132,"rows":40}"#))
            .await
            .unwrap();

        assert_eq!(
            next_terminal_input(&mut session.terminal_input).await,
            Some(TerminalInput::Resize(TerminalSize {
                columns: NonZeroU16::new(132).unwrap(),
                rows: NonZeroU16::new(40).unwrap(),
            }))
        );
    }

    #[tokio::test]
    async fn serve_terminal_websocket_incorrect_message_sends_error_and_closes_with_1008() {
        for (message, status) in [
            (r#"{"type":"resize","cols":0,"rows":24}"#, 422),
            (r#"{"type":"bell"}"#, 400),
            ("not JSON", 400),
        ] {
            let mut session = open_terminal_session().await;

            session.browser.send(Frame::text(message)).await.unwrap();

            let error_frame = next_frame(&mut session.browser).await;
            assert_eq!(
                text_frame_json(&error_frame)["error"]["status"],
                status,
                "{message}"
            );
            assert_matches!(
                next_frame(&mut session.browser).await,
                Frame::Close(Some(BrowserCloseFrame {
                    code: CloseCode::Policy,
                    ..
                })),
                "{message}"
            );
        }
    }

    #[tokio::test]
    async fn serve_terminal_websocket_exit_sends_exit_status_and_closes_with_1000() {
        let mut session = open_terminal_session().await;

        session
            .terminal_output
            .send(TerminalEvent::Exit(TerminalExit::Status(3)))
            .unwrap();

        assert_eq!(
            text_frame_json(&next_frame(&mut session.browser).await),
            json!({ "type": "exit_status", "status": 3 })
        );
        assert_matches!(
            next_frame(&mut session.browser).await,
            Frame::Close(Some(BrowserCloseFrame {
                code: CloseCode::Normal,
                ..
            }))
        );
        assert_eq!(next_terminal_input(&mut session.terminal_input).await, None);
    }

    #[tokio::test]
    async fn serve_terminal_websocket_machine_disconnect_sends_error_and_closes_with_1011() {
        let mut session = open_terminal_session().await;

        session
            .terminal_output
            .send(TerminalEvent::Exit(TerminalExit::Disconnected))
            .unwrap();

        let error_frame = text_frame_json(&next_frame(&mut session.browser).await);
        assert_eq!(error_frame["type"], "error");
        assert_eq!(error_frame["error"]["status"], 502);
        assert_matches!(
            next_frame(&mut session.browser).await,
            Frame::Close(Some(BrowserCloseFrame {
                code: CloseCode::Error,
                ..
            }))
        );
    }

    #[tokio::test]
    async fn serve_terminal_websocket_browser_close_drops_the_terminal() {
        let mut session = open_terminal_session().await;

        session.browser.close(None).await.unwrap();

        assert_eq!(next_terminal_input(&mut session.terminal_input).await, None);
    }

    #[tokio::test]
    async fn serve_terminal_websocket_message_over_the_limit_sends_error_and_closes_with_1009() {
        let mut session = open_terminal_session().await;
        // Two frames, each below the frame limit, so Paron reads all the data before it closes.
        // A socket that closes with unread data resets the connection and can lose the error.
        let frame_payload = vec![b'y'; MAX_MESSAGE_BYTES / 2 + 1];

        for (opcode, is_final) in [(Data::Binary, false), (Data::Continue, true)] {
            let frame = RawFrame::message(frame_payload.clone(), OpCode::Data(opcode), is_final);
            session.browser.send(Frame::Frame(frame)).await.unwrap();
        }

        let error_frame = next_frame(&mut session.browser).await;
        assert_eq!(text_frame_json(&error_frame)["error"]["status"], 413);
        assert_matches!(
            next_frame(&mut session.browser).await,
            Frame::Close(Some(BrowserCloseFrame {
                code: CloseCode::Size,
                ..
            }))
        );
        assert_eq!(next_terminal_input(&mut session.terminal_input).await, None);
    }

    #[tokio::test]
    async fn serve_terminal_websocket_reads_the_terminal_only_while_output_is_acknowledged() {
        const OUTPUT_CHUNK_BYTES: usize = 64 * 1024;
        const CHUNKS_UP_TO_THE_LIMIT: usize = MAX_UNACKNOWLEDGED_BYTES / OUTPUT_CHUNK_BYTES;
        let mut session = open_terminal_session().await;
        for _ in 0..CHUNKS_UP_TO_THE_LIMIT * 4 {
            session
                .terminal_output
                .send(TerminalEvent::Output(vec![b'y'; OUTPUT_CHUNK_BYTES]))
                .unwrap();
        }
        for _ in 0..CHUNKS_UP_TO_THE_LIMIT {
            next_frame(&mut session.browser).await;
        }
        // The server handles a later message, so it had the chance to read more output.
        session
            .browser
            .send(Frame::text(r#"{"type":"resize","cols":80,"rows":24}"#))
            .await
            .unwrap();
        next_terminal_input(&mut session.terminal_input).await;

        assert_eq!(
            session.terminal_events_read.load(Ordering::SeqCst),
            CHUNKS_UP_TO_THE_LIMIT
        );
        session
            .browser
            .send(Frame::text(format!(
                r#"{{"type":"ack","bytes":{MAX_UNACKNOWLEDGED_BYTES}}}"#
            )))
            .await
            .unwrap();
        assert_matches!(next_frame(&mut session.browser).await, Frame::Binary(_));
        assert!(session.terminal_events_read.load(Ordering::SeqCst) > CHUNKS_UP_TO_THE_LIMIT);
    }
}
