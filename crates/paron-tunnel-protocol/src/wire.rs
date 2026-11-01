//! The wire encodings of RFC §2: the version octet, the control-stream frames, the selector
//! encoding, and the tunnel-stream preamble and relay records.
//!
//! One codec for both transport mappings, which is what makes them equivalent (§12.7).
//!
//! Anything that does not decode is a protocol error (STM-3), surfaced as
//! `FrameError::Malformed`: the connection ends, and no message of §3 answers it.

use std::collections::BTreeSet;
use std::num::{NonZeroU16, NonZeroU64};
use std::path::PathBuf;

use secrecy::{ExposeSecret as _, SecretSlice};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::identity::AgentId;

/// The one protocol version this crate speaks (§2.1, VER-1).
///
/// It is therefore also the deployment's minimum (VER-6): a broker implements a contiguous range,
/// and this range is one long. Withdrawing a defective version means raising that minimum, which
/// here means shipping a crate that no longer implements it — there is no runtime switch, and a
/// correction to a published version is a new version rather than a change to this one.
pub const VERSION: u8 = 0x01;

/// The most octets a `JOIN` frame's token may carry (§2.4).
const MAX_TOKEN_OCTETS: usize = 4096;

/// The most octets a `unix-path` selector value may carry (§2.3).
const MAX_PATH_OCTETS: usize = 4096;

/// Frame type octets (§2.4).
const FRAME_CLAIM: u8 = 0x01;
const FRAME_ACCESS: u8 = 0x02;
const FRAME_JOIN: u8 = 0x03;
const FRAME_ANNOUNCE: u8 = 0x04;
const FRAME_PING: u8 = 0x05;
const FRAME_PONG: u8 = 0x06;

/// `ACCESS` verdict octets (§2.4).
const ACCESS_GRANTED: u8 = 0x01;
const ACCESS_DENIED: u8 = 0x02;

/// Selector type octets (§2.3).
const SELECTOR_TCP: u8 = 0x01;
const SELECTOR_UDP: u8 = 0x02;
const SELECTOR_UNIX: u8 = 0x03;
const SELECTOR_TCP_RANGE: u8 = 0x04;
const SELECTOR_UDP_RANGE: u8 = 0x05;
const SELECTOR_UNIX_ANY: u8 = 0x06;

/// The fields a selector encoding is measured in (§2.3).
const TYPE: usize = 1;
const PORT: usize = 2;
const PATH_LENGTH: usize = 2;

/// The one host designator PTP/1 defines: the agent's own host (§2.4, §2.5). A type octet rather
/// than a host — a later version defines designators whose value carries an address or a name
/// (§10.3).
const OWN_HOST: u8 = 0x01;

/// Tunnel-stream answer octets (§2.5).
const STREAM_ACCEPT: u8 = 0x01;
const STREAM_REFUSE: u8 = 0x02;

/// The broker's answer to an admission attempt, whether `CLAIM` or `JOIN` (RFC §3.2).
///
/// Two variants and never a third: `Denied` covers an unknown id, a known id with nothing bound
/// and the wrong key for a known id alike, SEC-7 requiring those to be indistinguishable in
/// content, in realisation and in timing (CNF-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// A private key is bound to this agent id, and it is the one proven on this connection.
    Granted,
    /// Everything else. An agent joining sends its token after this; an agent serving gives up.
    Denied,
}

/// What a `CLAIM` frame carries (§2.4): who the agent says it is, and the sequence that says this
/// claim is fresher than every one before it (POL-6, POL-10).
///
/// Proof of key possession is not a field: it is the handshake of the connection the frame
/// arrived on (TRN-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Claim {
    agent: AgentId,
    sequence: NonZeroU64,
}

impl Claim {
    /// `None` for a sequence of 0, which §2.1 excludes from the encoding entirely.
    pub(crate) fn new(agent: AgentId, sequence: u64) -> Option<Self> {
        Some(Self {
            agent,
            sequence: NonZeroU64::new(sequence)?,
        })
    }

    pub(crate) fn agent(&self) -> AgentId {
        self.agent
    }

    pub(crate) fn sequence(&self) -> u64 {
        self.sequence.get()
    }

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.agent.uuid().as_bytes());
        out.extend_from_slice(&self.sequence.get().to_be_bytes());
    }

    fn decode(payload: &[u8]) -> Result<Self, FrameError> {
        const WRONG_LENGTH: &str = "CLAIM is exactly 24 octets";

        let mut rest = payload;
        let agent = AgentId::new(uuid::Uuid::from_bytes(
            take(&mut rest).map_err(|_| FrameError::Malformed(WRONG_LENGTH))?,
        ));
        let sequence =
            u64::from_be_bytes(take(&mut rest).map_err(|_| FrameError::Malformed(WRONG_LENGTH))?);

        if !rest.is_empty() {
            return Err(FrameError::Malformed(WRONG_LENGTH));
        }

        Self::new(agent, sequence).ok_or(FrameError::Malformed("a sequence is never 0"))
    }
}

/// What a tunnel stream terminates at (§1.2, §2.3): a selector type that names one service.
///
/// Every variant names one service, which §2.3 makes the condition for appearing in a
/// tunnel-stream preamble. The three types that name a set of selectors instead are
/// [`Announced`], which is also how a `Selector` reaches an announced set (§3.1). So a preamble
/// cannot carry a set-naming type and POL-8 cannot be asked about one. Selector types beyond the
/// six PTP/1 defines are reserved (§10.3).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Selector {
    /// A TCP service on the designated host. Port 0 is not a selector.
    Tcp(NonZeroU16),
    /// A UDP service on the designated host, reached as framed records (§2.5).
    Udp(NonZeroU16),
    /// A UNIX-domain socket on the designated host, by path.
    Unix(UnixPath),
}

impl Selector {
    pub fn tcp(port: u16) -> Option<Self> {
        NonZeroU16::new(port).map(Self::Tcp)
    }

    pub fn udp(port: u16) -> Option<Self> {
        NonZeroU16::new(port).map(Self::Udp)
    }

    pub fn unix(path: impl Into<PathBuf>) -> Option<Self> {
        UnixPath::new(path).map(Self::Unix)
    }

    fn encoded_len(&self) -> usize {
        match self {
            Self::Tcp(_) | Self::Udp(_) => TYPE + PORT,
            Self::Unix(path) => TYPE + PATH_LENGTH + path.as_bytes().len(),
        }
    }

    /// §2.3: a type octet, then a value whose form the type fixes.
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Tcp(port) => {
                out.push(SELECTOR_TCP);
                out.extend_from_slice(&port.get().to_be_bytes());
            }
            Self::Udp(port) => {
                out.push(SELECTOR_UDP);
                out.extend_from_slice(&port.get().to_be_bytes());
            }
            Self::Unix(path) => {
                out.push(SELECTOR_UNIX);
                out.extend_from_slice(&path.length.to_be_bytes());
                out.extend_from_slice(path.as_bytes());
            }
        }
    }

    /// The one place a service-naming selector is validated: port 0, path length and the 0x00
    /// octet are refused here whether the encoding arrived in an `ANNOUNCE` payload or as a
    /// tunnel-stream preamble ([`read_stream_preamble`] reads the octets and hands them here).
    ///
    /// A further service-naming type is added here, and [`Announced::decode`] then carries it
    /// without an edit of its own.
    fn decode(rest: &mut &[u8]) -> Result<Self, FrameError> {
        match take::<1>(rest)?[0] {
            kind @ (SELECTOR_TCP | SELECTOR_UDP) => {
                let port = NonZeroU16::new(u16::from_be_bytes(take(rest)?))
                    .ok_or(FrameError::Malformed("port 0 is not a selector"))?;

                Ok(match kind {
                    SELECTOR_TCP => Self::Tcp(port),
                    _ => Self::Udp(port),
                })
            }
            SELECTOR_UNIX => {
                let length = usize::from(u16::from_be_bytes(take(rest)?));
                let bytes = take_n(rest, length)?.to_vec();

                UnixPath::from_bytes(bytes)
                    .map(Self::Unix)
                    .ok_or(FrameError::Malformed(
                        "a unix-path is 1 to 4096 octets, none of them 0x00",
                    ))
            }
            // Known types, and §2.3 bars them from a preamble, so "unknown selector type" would
            // be false. The distinction is only ever a diagnostic: `Selector` has no set-naming
            // variant, so neither arm can produce one.
            SELECTOR_TCP_RANGE | SELECTOR_UDP_RANGE | SELECTOR_UNIX_ANY => Err(
                FrameError::Malformed("a selector that names no service opens no stream"),
            ),
            _ => Err(FrameError::Malformed("unknown selector type")),
        }
    }
}

/// A member of an announced set (§3.1): any selector type PTP/1 defines.
///
/// [`One`](Announced::One) carries the types that name one service, so a service-naming type
/// belongs in [`Selector`] and reaches an announcement through this variant. The other three
/// name a set of selectors and no service (§2.3), which is why they are here and not there.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Announced {
    /// One service, announced by the selector that names it.
    One(Selector),
    /// Every [`Selector::Tcp`] the range spans.
    TcpRange(PortRange),
    /// Every [`Selector::Udp`] the range spans.
    UdpRange(PortRange),
    /// Every [`Selector::Unix`], whatever its path.
    UnixAny,
}

impl Announced {
    /// Whether `selector` is one of the selectors this member identifies (§2.3).
    ///
    /// [`One`](Announced::One) identifies its own selector and nothing else, which is §3.1's "a
    /// selector naming a service stands for itself".
    pub fn identifies(&self, selector: &Selector) -> bool {
        match self {
            Self::One(one) => one == selector,
            Self::TcpRange(range) => {
                matches!(selector, Selector::Tcp(port) if range.covers(*port))
            }
            Self::UdpRange(range) => {
                matches!(selector, Selector::Udp(port) if range.covers(*port))
            }
            Self::UnixAny => matches!(selector, Selector::Unix(_)),
        }
    }

    fn encoded_len(&self) -> usize {
        match self {
            Self::One(selector) => selector.encoded_len(),
            Self::TcpRange(_) | Self::UdpRange(_) => TYPE + PORT + PORT,
            Self::UnixAny => TYPE,
        }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::One(selector) => selector.encode(out),
            Self::TcpRange(range) => {
                out.push(SELECTOR_TCP_RANGE);
                range.encode(out);
            }
            Self::UdpRange(range) => {
                out.push(SELECTOR_UDP_RANGE);
                range.encode(out);
            }
            Self::UnixAny => out.push(SELECTOR_UNIX_ANY),
        }
    }

    /// The three set-naming types are read here, and everything else is [`Selector::decode`]'s,
    /// which is what keeps one type-octet space in one place: a type octet this does not claim
    /// is a selector's, whether PTP/1 defines it or a later version does.
    fn decode(rest: &mut &[u8]) -> Result<Self, FrameError> {
        match rest.first().copied() {
            Some(kind @ (SELECTOR_TCP_RANGE | SELECTOR_UDP_RANGE)) => {
                *rest = &rest[1..];

                let low = u16::from_be_bytes(take(rest)?);
                let high = u16::from_be_bytes(take(rest)?);
                let range = PortRange::new(low, high).ok_or(FrameError::Malformed(
                    "a port range is two ports, neither 0, the high not below the low",
                ))?;

                Ok(match kind {
                    SELECTOR_TCP_RANGE => Self::TcpRange(range),
                    _ => Self::UdpRange(range),
                })
            }
            Some(SELECTOR_UNIX_ANY) => {
                *rest = &rest[1..];

                Ok(Self::UnixAny)
            }
            // Every other type octet is a selector's, including one a later version defines.
            // An empty `rest` lands here too, and is the truncated payload `Selector::decode`
            // reports.
            _ => Selector::decode(rest).map(Self::One),
        }
    }
}

impl From<Selector> for Announced {
    fn from(selector: Selector) -> Self {
        Self::One(selector)
    }
}

/// The value a `tcp-port-range` or a `udp-port-range` carries (§2.3): a low port and a high port,
/// neither 0 and the high not below the low.
///
/// Held as the pair that travels, so the invariant the encoding states is the invariant the type
/// holds: [`new`](PortRange::new) is the only way in, and it is the only place the pair is
/// checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortRange {
    low: NonZeroU16,
    high: NonZeroU16,
}

impl PortRange {
    pub fn new(low: u16, high: u16) -> Option<Self> {
        let (low, high) = (NonZeroU16::new(low)?, NonZeroU16::new(high)?);

        (low <= high).then_some(Self { low, high })
    }

    /// Every port of the range's type: the pair 1 and 65535 (§2.3).
    pub fn every() -> Self {
        Self {
            low: NonZeroU16::MIN,
            high: NonZeroU16::MAX,
        }
    }

    fn covers(self, port: NonZeroU16) -> bool {
        (self.low..=self.high).contains(&port)
    }

    fn encode(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.low.get().to_be_bytes());
        out.extend_from_slice(&self.high.get().to_be_bytes());
    }
}

impl std::fmt::Display for PortRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.low, self.high)
    }
}

impl std::fmt::Display for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp(port) => write!(f, "tcp-port {port}"),
            Self::Udp(port) => write!(f, "udp-port {port}"),
            Self::Unix(path) => write!(f, "unix-path {}", path.path().display()),
        }
    }
}

impl std::fmt::Display for Announced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::One(selector) => selector.fmt(f),
            Self::TcpRange(range) => write!(f, "tcp-port-range {range}"),
            Self::UdpRange(range) => write!(f, "udp-port-range {range}"),
            Self::UnixAny => f.write_str("unix-path-any"),
        }
    }
}

/// A `unix-path` selector value: 1 to 4096 octets, none of them 0x00 (§2.3).
///
/// Held as the octets that travel, so equality is equality of the encoding, which is what §2.3
/// defines selector equality to be.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixPath {
    octets: Vec<u8>,
    length: u16,
}

impl UnixPath {
    pub fn new(path: impl Into<PathBuf>) -> Option<Self> {
        let path = path.into().into_os_string();
        let bytes = path.as_encoded_bytes().to_vec();

        Self::from_bytes(bytes)
    }

    /// §2.3's rule, and the whole of what a UNIX path is: any octets but 0x00, which terminates
    /// the kernel's `sun_path`. Not UTF-8 — a filesystem path is a byte string, and refusing the
    /// ones that are not text would refuse sockets the host itself accepts.
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        let length = u16::try_from(bytes.len()).ok()?;

        ((1..=MAX_PATH_OCTETS).contains(&bytes.len()) && !bytes.contains(&0)).then_some(Self {
            octets: bytes,
            length,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.octets
    }

    #[cfg(unix)]
    pub fn path(&self) -> PathBuf {
        use std::os::unix::ffi::OsStringExt as _;

        PathBuf::from(std::ffi::OsString::from_vec(self.octets.clone()))
    }

    /// Off-unix there are no UNIX-domain sockets to name, so a path is only ever displayed and
    /// a lossy reading costs nothing.
    #[cfg(not(unix))]
    pub fn path(&self) -> PathBuf {
        PathBuf::from(String::from_utf8_lossy(&self.octets).into_owned())
    }
}

/// The set an `ANNOUNCE` frame carries (§2.4, §3.1): every selector the agent expects to serve.
/// Informational — a broker reports it to its operator and derives nothing else from it (STM-13).
///
/// A set by construction, which is how the encoding's no-duplicates rule holds on the way out; on
/// the way in a duplicate is a malformed frame (STM-3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnnounceSet(BTreeSet<Announced>);

impl AnnounceSet {
    pub fn new(members: impl IntoIterator<Item = Announced>) -> Self {
        Self(members.into_iter().collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Announced> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// An `ANNOUNCE` occupies one frame, so the announced set is bounded by the frame's payload
    /// length (§2.4). Asked where an agent assembles the set rather than where the frame is
    /// encoded: one that cannot travel has to be caught before a frame is built for it.
    pub fn fits_one_frame(&self) -> bool {
        u16::try_from(self.encoded_len()).is_ok()
    }

    /// The octets [`encode`](AnnounceSet::encode) writes.
    fn encoded_len(&self) -> usize {
        const DESIGNATOR: usize = 1;
        const COUNT: usize = 2;

        DESIGNATOR + COUNT + self.iter().map(Announced::encoded_len).sum::<usize>()
    }

    #[expect(clippy::expect_used)]
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(OWN_HOST);
        out.extend_from_slice(
            &u16::try_from(self.len())
                .expect("an AnnounceSet that fits one frame holds fewer than 2^16 selectors")
                .to_be_bytes(),
        );

        for member in self.iter() {
            member.encode(out);
        }
    }

    fn decode(payload: &[u8]) -> Result<Self, FrameError> {
        let mut rest = payload;

        if take::<1>(&mut rest)?[0] != OWN_HOST {
            return Err(FrameError::Malformed("unknown host designator"));
        }

        let count = u16::from_be_bytes(take(&mut rest)?);
        let mut members = BTreeSet::new();

        for _ in 0..count {
            if !members.insert(Announced::decode(&mut rest)?) {
                return Err(FrameError::Malformed("a selector appears more than once"));
            }
        }

        if !rest.is_empty() {
            return Err(FrameError::Malformed(
                "ANNOUNCE carries octets beyond its count",
            ));
        }

        Ok(Self(members))
    }
}

/// One frame of the control stream (§2.4).
///
/// `Join` wraps its token in [`SecretSlice`] so the one confidential value the protocol carries
/// cannot be `Debug`-printed or logged by accident (TRN-5).
#[derive(Debug)]
pub enum Frame {
    Claim(Claim),
    Access(Access),
    Join(SecretSlice<u8>),
    Announce(AnnounceSet),
    Ping,
    Pong,
}

impl Frame {
    fn type_octet(&self) -> u8 {
        match self {
            Self::Claim(_) => FRAME_CLAIM,
            Self::Access(_) => FRAME_ACCESS,
            Self::Join(_) => FRAME_JOIN,
            Self::Announce(_) => FRAME_ANNOUNCE,
            Self::Ping => FRAME_PING,
            Self::Pong => FRAME_PONG,
        }
    }

    fn encode_payload(&self, out: &mut Vec<u8>) {
        match self {
            Self::Claim(claim) => claim.encode(out),
            Self::Access(access) => out.push(match access {
                Access::Granted => ACCESS_GRANTED,
                Access::Denied => ACCESS_DENIED,
            }),
            Self::Join(token) => out.extend_from_slice(token.expose_secret()),
            Self::Announce(set) => set.encode(out),
            Self::Ping | Self::Pong => {}
        }
    }

    /// The payload is owned because `JOIN` keeps it: the token goes straight into a
    /// [`SecretSlice`] rather than being copied out of a borrow (TRN-5).
    fn decode(type_octet: u8, payload: Vec<u8>) -> Result<Self, FrameError> {
        match type_octet {
            FRAME_CLAIM => Claim::decode(&payload).map(Self::Claim),
            FRAME_ACCESS => match payload.as_slice() {
                [ACCESS_GRANTED] => Ok(Self::Access(Access::Granted)),
                [ACCESS_DENIED] => Ok(Self::Access(Access::Denied)),
                _ => Err(FrameError::Malformed("ACCESS carries one verdict octet")),
            },
            FRAME_JOIN => {
                if payload.is_empty() || payload.len() > MAX_TOKEN_OCTETS {
                    return Err(FrameError::Malformed("JOIN carries 1 to 4096 octets"));
                }

                Ok(Self::Join(SecretSlice::from(payload)))
            }
            FRAME_ANNOUNCE => AnnounceSet::decode(&payload).map(Self::Announce),
            FRAME_PING => Self::without_payload(&payload, Self::Ping, "PING carries no payload"),
            FRAME_PONG => Self::without_payload(&payload, Self::Pong, "PONG carries no payload"),
            _ => Err(FrameError::Malformed("unknown frame type")),
        }
    }

    fn without_payload(
        payload: &[u8],
        frame: Self,
        complaint: &'static str,
    ) -> Result<Self, FrameError> {
        payload
            .is_empty()
            .then_some(frame)
            .ok_or(FrameError::Malformed(complaint))
    }
}

/// Why a read off the control stream failed: the connection went, or the peer spoke something
/// that is not this protocol, which ends it (STM-3).
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("malformed frame: {0}")]
    Malformed(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The version octet, once per connection: the first octet the agent writes on the control
/// stream, before any frame (TRN-11, VER-1).
///
/// Frames carry no version of their own, so a connection runs for its whole life under the
/// version its first octet declared; a broker answers one outside its range with
/// `ACCESS`(Denied) and ends the connection rather than reinterpreting or renegotiating (VER-2).
///
/// VER-7 binds every future version to four rules, and three of them are visible from here: this
/// octet goes first, nothing of the protocol is written before the broker's identity is proven
/// (TRN-2, and the agent's TLS configuration is what enforces it), and a version outside the
/// range is answered by this document's `ACCESS` frame and the connection then ends. The fourth,
/// VER-3's retry discipline, has nothing to govern while there is one version.
pub async fn write_version(control: &mut (impl AsyncWrite + Unpin)) -> std::io::Result<()> {
    control.write_all(&[VERSION]).await?;
    control.flush().await
}

/// The peer's declared version, read before any frame. A value outside the broker's range is a
/// denial, not a decode failure (VER-2).
pub async fn read_version(control: &mut (impl AsyncRead + Unpin)) -> std::io::Result<u8> {
    control.read_u8().await
}

/// One frame: type octet, 2-octet payload length, payload (§2.4).
///
/// A payload beyond 65535 octets has no encoding, so it fails here rather than travelling
/// truncated. Only `ANNOUNCE` can approach the bound, and an agent asks
/// [`AnnounceSet::fits_one_frame`] before building one, so reaching this is a caller's bug.
pub async fn write_frame(
    control: &mut (impl AsyncWrite + Unpin),
    frame: &Frame,
) -> std::io::Result<()> {
    const HEADER: usize = 3;

    // The header is written before the payload it measures, so its length goes in as a placeholder
    // and is patched once the payload is there.
    let mut encoded = vec![frame.type_octet(), 0, 0];
    frame.encode_payload(&mut encoded);

    let length = u16::try_from(encoded.len() - HEADER)
        .map_err(|_| std::io::Error::other("frame payload exceeds one frame"))?;
    encoded[1..HEADER].copy_from_slice(&length.to_be_bytes());

    control.write_all(&encoded).await?;
    control.flush().await
}

pub async fn read_frame(control: &mut (impl AsyncRead + Unpin)) -> Result<Frame, FrameError> {
    let type_octet = control.read_u8().await?;
    let length = usize::from(control.read_u16().await?);

    // The length is 16-bit, so this allocates at most 64 KiB whatever the type octet claims;
    // each type then re-checks its own payload.
    let mut payload = vec![0u8; length];
    control.read_exact(&mut payload).await?;

    Frame::decode(type_octet, payload)
}

/// The next `N` octets, or a truncated payload. Fixed-size so the caller can hand them straight
/// to `from_be_bytes` rather than converting and asserting it cannot fail.
fn take<const N: usize>(rest: &mut &[u8]) -> Result<[u8; N], FrameError> {
    let (taken, remaining) = rest
        .split_first_chunk::<N>()
        .ok_or(FrameError::Malformed("truncated payload"))?;
    *rest = remaining;

    Ok(*taken)
}

/// The next `n` octets, for a field whose length its own header carries.
fn take_n<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], FrameError> {
    let (taken, remaining) = rest
        .split_at_checked(n)
        .ok_or(FrameError::Malformed("truncated payload"))?;
    *rest = remaining;

    Ok(taken)
}

/// The preamble that opens a tunnel stream: a host designator and one selector (§2.5). The
/// broker writes it.
pub async fn write_stream_preamble(
    stream: &mut (impl AsyncWrite + Unpin),
    selector: &Selector,
) -> std::io::Result<()> {
    let mut encoded = Vec::with_capacity(1 + selector.encoded_len());
    encoded.push(OWN_HOST);
    selector.encode(&mut encoded);

    stream.write_all(&encoded).await?;
    stream.flush().await
}

/// Read the selector a just-opened tunnel stream names (§2.5). The agent reads it.
///
/// A tunnel stream carries the service's own bytes straight after the preamble, so this pulls
/// exactly the octets the selector type calls for and not one more. What they mean is
/// [`Selector::decode`]'s to say, so the two places a selector arrives agree by construction.
pub async fn read_stream_preamble(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<Selector, FrameError> {
    if stream.read_u8().await? != OWN_HOST {
        return Err(FrameError::Malformed("unknown host designator"));
    }

    let kind = stream.read_u8().await?;
    let mut encoded = vec![kind];

    match kind {
        SELECTOR_TCP | SELECTOR_UDP => {
            encoded.extend_from_slice(&stream.read_u16().await?.to_be_bytes());
        }
        SELECTOR_UNIX => {
            let length = stream.read_u16().await?;
            encoded.extend_from_slice(&length.to_be_bytes());

            let at = encoded.len();
            encoded.resize(at + usize::from(length), 0);
            stream.read_exact(&mut encoded[at..]).await?;
        }
        // §2.5 excludes these three from a preamble. They get their own arm rather than falling
        // to the one below, because §2.3 defines all three and "unknown selector type" would
        // therefore be false.
        SELECTOR_TCP_RANGE | SELECTOR_UDP_RANGE | SELECTOR_UNIX_ANY => {
            return Err(FrameError::Malformed(
                "a selector that names no service opens no stream",
            ));
        }
        _ => return Err(FrameError::Malformed("unknown selector type")),
    }

    Selector::decode(&mut &encoded[..])
}

/// The agent's answer to a stream preamble: `STREAM_ACCEPT` as 0x01, `STREAM_REFUSE` as 0x02
/// (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamAnswer {
    Accept,
    Refuse,
}

pub async fn write_stream_answer(
    stream: &mut (impl AsyncWrite + Unpin),
    answer: StreamAnswer,
) -> std::io::Result<()> {
    stream
        .write_all(&[match answer {
            StreamAnswer::Accept => STREAM_ACCEPT,
            StreamAnswer::Refuse => STREAM_REFUSE,
        }])
        .await?;
    stream.flush().await
}

pub async fn read_stream_answer(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<StreamAnswer, FrameError> {
    match stream.read_u8().await? {
        STREAM_ACCEPT => Ok(StreamAnswer::Accept),
        STREAM_REFUSE => Ok(StreamAnswer::Refuse),
        _ => Err(FrameError::Malformed("a stream answer is 0x01 or 0x02")),
    }
}

/// One datagram, crossing a `udp-port` stream as a length-prefixed record (§2.5). A record of
/// length 0 is an empty datagram, which UDP permits.
pub async fn write_record(
    stream: &mut (impl AsyncWrite + Unpin),
    datagram: &[u8],
) -> std::io::Result<()> {
    let length = u16::try_from(datagram.len())
        .map_err(|_| std::io::Error::other("a datagram exceeds one record"))?;

    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(datagram).await?;
    stream.flush().await
}

/// The next record, into `datagram`, which is resized to it. Reusing one buffer keeps a relay
/// from allocating per datagram.
pub async fn read_record(
    stream: &mut (impl AsyncRead + Unpin),
    datagram: &mut Vec<u8>,
) -> std::io::Result<()> {
    let length = usize::from(stream.read_u16().await?);

    datagram.clear();
    datagram.resize(length, 0);
    stream.read_exact(datagram).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    async fn write_then_read_frame(frame: Frame) -> Frame {
        let mut frame_bytes = Vec::new();
        write_frame(&mut frame_bytes, &frame).await.unwrap();

        read_frame(&mut frame_bytes.as_slice()).await.unwrap()
    }

    fn agent_id() -> AgentId {
        AgentId::new(uuid::Uuid::now_v7())
    }

    #[tokio::test]
    async fn round_trip_claim_frame_keeps_claim() {
        let claim = Claim::new(agent_id(), 7).unwrap();

        let decoded = write_then_read_frame(Frame::Claim(claim)).await;

        assert_matches!(decoded, Frame::Claim(read_claim) if read_claim == claim);
    }

    #[tokio::test]
    async fn round_trip_announce_frame_keeps_every_member_type() {
        let announce_set = AnnounceSet::new([
            Selector::tcp(22).unwrap().into(),
            Selector::udp(53).unwrap().into(),
            Selector::unix("/run/paron.sock").unwrap().into(),
            Announced::TcpRange(PortRange::every()),
            Announced::UdpRange(PortRange::new(8000, 8080).unwrap()),
            Announced::UnixAny,
        ]);

        let decoded = write_then_read_frame(Frame::Announce(announce_set.clone())).await;

        assert_matches!(decoded, Frame::Announce(read_set) if read_set == announce_set);
    }

    #[tokio::test]
    async fn round_trip_announce_frame_keeps_empty_set() {
        let decoded = write_then_read_frame(Frame::Announce(AnnounceSet::default())).await;

        assert_matches!(decoded, Frame::Announce(read_set) if read_set == AnnounceSet::default());
    }

    #[tokio::test]
    async fn read_frame_malformed_returns_malformed_error() {
        let mut zero_sequence = vec![0x01, 0x00, 24];
        zero_sequence.extend_from_slice(agent_id().uuid().as_bytes());
        zero_sequence.extend_from_slice(&0u64.to_be_bytes());

        let malformed_frames: [(&str, Vec<u8>); 9] = [
            ("claim with sequence 0", zero_sequence),
            (
                "claim of the wrong length",
                vec![0x01, 0x00, 0x02, 0xAA, 0xBB],
            ),
            ("unknown frame type", vec![0x7F, 0x00, 0x00]),
            (
                "announce with a duplicate selector",
                vec![
                    0x04, 0x00, 0x09, 0x01, 0x00, 0x02, 0x01, 0x00, 0x16, 0x01, 0x00, 0x16,
                ],
            ),
            (
                "announce with port 0",
                vec![0x04, 0x00, 0x06, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00],
            ),
            (
                "announce with an unknown host designator",
                vec![0x04, 0x00, 0x03, 0x02, 0x00, 0x00],
            ),
            (
                "announce with a range that starts at port 0",
                vec![
                    0x04, 0x00, 0x08, 0x01, 0x00, 0x01, 0x04, 0x00, 0x00, 0x00, 0x16,
                ],
            ),
            (
                "announce with a reversed range",
                vec![
                    0x04, 0x00, 0x08, 0x01, 0x00, 0x01, 0x04, 0x00, 0x17, 0x00, 0x16,
                ],
            ),
            ("ping with a payload", vec![0x05, 0x00, 0x01, 0x00]),
        ];

        for (case, frame_bytes) in malformed_frames {
            let decoded = read_frame(&mut frame_bytes.as_slice()).await;

            assert_matches!(decoded, Err(FrameError::Malformed(_)), "{case}");
        }
    }

    #[tokio::test]
    async fn read_record_of_empty_datagram_clears_buffer() {
        let mut record_bytes = Vec::new();
        write_record(&mut record_bytes, &[]).await.unwrap();
        let mut datagram = vec![0xAA; 8];

        read_record(&mut record_bytes.as_slice(), &mut datagram)
            .await
            .unwrap();

        assert_eq!(datagram, Vec::<u8>::new());
    }

    #[tokio::test]
    async fn round_trip_stream_preamble_keeps_unix_path() {
        let selector = Selector::unix("/tmp/echo.sock").unwrap();
        let mut preamble_bytes = Vec::new();
        write_stream_preamble(&mut preamble_bytes, &selector)
            .await
            .unwrap();

        let decoded = read_stream_preamble(&mut preamble_bytes.as_slice()).await;

        assert_matches!(decoded, Ok(read_selector) if read_selector == selector);
    }

    #[tokio::test]
    async fn read_stream_preamble_of_unknown_host_designator_returns_malformed_error() {
        let preamble_bytes = [0x02, 0x01, 0x00, 0x16];

        let decoded = read_stream_preamble(&mut preamble_bytes.as_slice()).await;

        assert_matches!(decoded, Err(FrameError::Malformed(_)));
    }

    #[tokio::test]
    async fn read_stream_preamble_of_set_type_returns_malformed_error() {
        let set_type_preambles = [
            (
                "tcp-port-range",
                [0x01, 0x04, 0x00, 0x01, 0xff, 0xff].as_slice(),
            ),
            (
                "udp-port-range",
                [0x01, 0x05, 0x1f, 0x40, 0x1f, 0x90].as_slice(),
            ),
            ("unix-any", [0x01, 0x06].as_slice()),
        ];

        for (case, mut preamble_bytes) in set_type_preambles {
            let decoded = read_stream_preamble(&mut preamble_bytes).await;

            assert_matches!(
                decoded,
                Err(FrameError::Malformed(
                    "a selector that names no service opens no stream"
                )),
                "{case}"
            );
        }
    }

    #[test]
    fn identifies_of_tcp_range_matches_tcp_only() {
        let every_tcp = Announced::TcpRange(PortRange::every());

        let identified = [
            Selector::tcp(22).unwrap(),
            Selector::udp(22).unwrap(),
            Selector::unix("/run/x.sock").unwrap(),
        ]
        .map(|selector| every_tcp.identifies(&selector));

        assert_eq!(identified, [true, false, false]);
    }

    #[test]
    fn identifies_of_one_port_matches_that_port_only() {
        let ssh: Announced = Selector::tcp(22).unwrap().into();

        let identified = [Selector::tcp(22), Selector::tcp(23)]
            .map(|selector| ssh.identifies(&selector.unwrap()));

        assert_eq!(identified, [true, false]);
    }

    #[test]
    fn identifies_of_unix_any_matches_unix_only() {
        let identified = [
            Selector::unix("/run/x.sock").unwrap(),
            Selector::tcp(22).unwrap(),
        ]
        .map(|selector| Announced::UnixAny.identifies(&selector));

        assert_eq!(identified, [true, false]);
    }

    #[test]
    fn identifies_of_port_range_matches_from_low_to_high_port() {
        let range = Announced::UdpRange(PortRange::new(8000, 8080).unwrap());

        let identified = [1, 7999, 8000, 8040, 8080, 8081, 65535]
            .map(|port| range.identifies(&Selector::udp(port).unwrap()));

        assert_eq!(identified, [false, false, true, true, true, false, false]);
    }

    #[test]
    fn new_port_range_with_zero_or_reversed_ports_returns_none() {
        let ranges = [(0, 65535), (1, 0), (23, 22)].map(|(low, high)| PortRange::new(low, high));

        assert_eq!(ranges, [None; 3]);
    }

    #[test]
    fn new_port_range_of_one_port_returns_range() {
        let range = PortRange::new(22, 22);

        assert_matches!(range, Some(_));
    }

    #[test]
    fn from_bytes_with_nul_returns_none() {
        let path = UnixPath::from_bytes(b"/a\0b".to_vec());

        assert_eq!(path, None);
    }

    #[cfg(unix)]
    #[test]
    fn path_of_non_utf8_octets_keeps_octets() {
        use std::os::unix::ffi::OsStrExt as _;

        let unix_path = UnixPath::from_bytes(b"/tmp/\xff\xfe.sock".to_vec()).unwrap();

        let path = unix_path.path();

        assert_eq!(path.as_os_str().as_bytes(), b"/tmp/\xff\xfe.sock");
    }

    // 17 paths of 4000 octets do not fit in the 65535 octets of one frame.
    #[test]
    fn fits_one_frame_of_oversized_set_returns_false() {
        let oversized_set = AnnounceSet::new((0..17).map(|path_number| {
            Selector::unix(format!("/{}/{path_number}", "x".repeat(4000)))
                .unwrap()
                .into()
        }));

        let fits = oversized_set.fits_one_frame();

        assert!(!fits);
    }
}
