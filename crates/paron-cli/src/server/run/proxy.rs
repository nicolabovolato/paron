use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::http::{HeaderMap, HeaderName};
use forwarded_header_value::{ForwardedHeaderValue, ForwardedStanza};
use ipnet::{IpNet, Ipv4Net};

use tokio::io::AsyncReadExt as _;
use tokio::net::TcpStream;

pub const FORWARDED: HeaderName = HeaderName::from_static("forwarded");
pub const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
pub const X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
pub const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
pub const X_REAL_IP: HeaderName = HeaderName::from_static("x-real-ip");

pub const PROXY_FORWARDING_HEADERS: [HeaderName; 5] = [
    FORWARDED,
    X_FORWARDED_FOR,
    X_FORWARDED_HOST,
    X_FORWARDED_PROTO,
    X_REAL_IP,
];

/// PROXY protocol V1 signature (HAProxy proxy protocol, 2.1).
const PROXY_V1_SIGNATURE: &[u8] = b"PROXY ";
/// PROXY protocol V2 signature (HAProxy proxy protocol, 2.2).
const PROXY_V2_SIGNATURE: &[u8] = b"\r\n\r\n\0\r\nQUIT\n";

const PROXY_V1_PAYLOAD_MAX_LENGTH: usize = 108;

const SIGNATURE_LEN: usize = if PROXY_V1_SIGNATURE.len() > PROXY_V2_SIGNATURE.len() {
    PROXY_V1_SIGNATURE.len()
} else {
    PROXY_V2_SIGNATURE.len()
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NetworkAddress(IpNet);

impl NetworkAddress {
    pub fn contains(self, ip: IpAddr) -> bool {
        self.0.contains(&Self::normalise_ip(ip))
    }

    /// Tries to convert an IPv6 address to IPv4, and returns the original if it is not mappable.
    fn normalise_ip(address: IpAddr) -> IpAddr {
        match address {
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => address,
            },
            IpAddr::V4(_) => address,
        }
    }

    /// Tries to convert an IPv6 network address to IPv4, and returns the original if it is not mappable.
    fn normalise_network_address(block: IpNet) -> Option<IpNet> {
        let IpNet::V6(v6) = block else {
            return Some(block);
        };

        let Some(base) = v6.addr().to_ipv4_mapped() else {
            return Some(block);
        };

        let prefix = v6.prefix_len().checked_sub(96)?;

        Some(IpNet::V4(Ipv4Net::new(base, prefix).ok()?))
    }
}

impl std::fmt::Display for NetworkAddress {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(out)
    }
}

impl std::str::FromStr for NetworkAddress {
    type Err = String;

    fn from_str(raw: &str) -> std::result::Result<Self, Self::Err> {
        let raw = raw.trim();

        let block = if raw.contains('/') {
            raw.parse::<IpNet>().map_err(|_| ())
        } else {
            raw.parse::<IpAddr>().map(IpNet::from).map_err(|_| ())
        }
        .map_err(|()| format!("{raw:?} is not an ip address or an ip network address"))?;

        Self::normalise_network_address(block)
            .map(Self)
            .ok_or_else(|| {
                format!(
                    "{raw:?} is an IPv4 network address written as IPv6 and needs a prefix length between 96 and 128"
                )
            })
    }
}

#[derive(Clone, Default)]
pub struct Networks(Arc<[NetworkAddress]>);

impl Networks {
    pub fn new(networks: &[NetworkAddress]) -> Self {
        Self(networks.into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains(&self, address: IpAddr) -> bool {
        self.0.iter().any(|network| network.contains(address))
    }
}

impl std::fmt::Display for Networks {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (position, network) in self.0.iter().enumerate() {
            if position > 0 {
                out.write_str(", ")?;
            }

            write!(out, "{network}")?;
        }

        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct Proxies {
    pub l4: L4Proxies,
    pub l7: L7Proxies,
}

#[derive(Clone, Default, derive_more::Deref)]
pub struct L4Proxies(Networks);

impl L4Proxies {
    pub fn new(networks: &[NetworkAddress]) -> Self {
        Self(Networks::new(networks))
    }
}

#[derive(Clone, Default, derive_more::Deref)]
pub struct L7Proxies(Networks);

impl L7Proxies {
    pub fn new(networks: &[NetworkAddress]) -> Self {
        Self(Networks::new(networks))
    }
}

impl L7Proxies {
    pub fn client_from_headers(&self, headers: &HeaderMap) -> Option<IpAddr> {
        self.first_untrusted_or_last_trusted(&Self::x_forwarded_for(headers))
            .or_else(|| self.first_untrusted_or_last_trusted(&Self::forwarded(headers)))
            .or_else(|| Self::x_real_ip(headers))
    }

    /// Rightmost first, which is the order the chain is read in: the last entry is the hop that
    /// spoke to us, and each one to its left is what that hop was told.
    fn x_forwarded_for(headers: &HeaderMap) -> Vec<Option<IpAddr>> {
        headers
            .get_all(X_FORWARDED_FOR)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .map(|entry| entry.trim().parse().ok())
            .collect()
    }

    /// The same chain where a deployment sends RFC 7239 instead.
    fn forwarded(headers: &HeaderMap) -> Vec<Option<IpAddr>> {
        let mut chain = Vec::new();

        for value in headers.get_all(FORWARDED) {
            let Ok(value) = value.to_str() else {
                continue;
            };

            let Ok(parsed) = ForwardedHeaderValue::from_forwarded(value) else {
                continue;
            };

            chain.extend(parsed.iter().map(ForwardedStanza::forwarded_for_ip));
        }

        chain
    }

    fn x_real_ip(headers: &HeaderMap) -> Option<IpAddr> {
        headers.get(X_REAL_IP)?.to_str().ok()?.trim().parse().ok()
    }

    /// `as_sent` is the chain in the order the header carries it: a hop appends, and a proxy
    /// must not reorder what it forwards (RFC 9110 §5.3), so the first entry is the one added
    /// furthest from us and the last is the hop that spoke to us. This walks it the other way.
    fn first_untrusted_or_last_trusted(&self, as_sent: &[Option<IpAddr>]) -> Option<IpAddr> {
        let mut skipped = None;

        // An entry that does not parse ends the walk, because nothing to its left was vouched
        // for by anything we trust.
        for &entry in as_sent.iter().rev() {
            let Some(address) = entry else {
                return skipped;
            };

            if !self.contains(address) {
                return Some(address);
            }

            skipped = Some(address);
        }

        skipped
    }
}

pub fn report_untrusted_l4_peer(peer: SocketAddr, l4: &L4Proxies) {
    static UNTRUSTED_L4_PEER_REPORTED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    const MESSAGE: &str = "not a trusted L4 proxy, PROXY protocol forwarding is ignored";

    if l4.is_empty() {
        return;
    }

    if UNTRUSTED_L4_PEER_REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::debug!(%peer, trusted = %**l4, "{MESSAGE}");
    } else {
        tracing::warn!(%peer, trusted = %**l4, "{MESSAGE}");
    }
}

pub async fn client_from_proxy_protocol(
    tcp: &mut TcpStream,
) -> std::io::Result<Option<SocketAddr>> {
    let mut signature = [0u8; SIGNATURE_LEN];

    if tcp.peek(&mut signature).await? < SIGNATURE_LEN {
        return Ok(None);
    }

    match signature {
        seen if seen.starts_with(PROXY_V2_SIGNATURE) => read_proxy_protocol_v2(tcp).await,
        seen if seen.starts_with(PROXY_V1_SIGNATURE) => read_proxy_protocol_v1(tcp).await,
        _ => Ok(None),
    }
}

async fn read_proxy_protocol_v2(tcp: &mut TcpStream) -> std::io::Result<Option<SocketAddr>> {
    let mut header = [0u8; 16];
    tcp.read_exact(&mut header).await?;

    let command = header[12];
    let family = header[13];

    if command & 0xF0 != 0x20 {
        return Ok(None);
    }

    let length = usize::from(u16::from_be_bytes([header[14], header[15]]));

    let mut addresses = vec![0u8; length];
    tcp.read_exact(&mut addresses).await?;

    if command != 0x21 {
        return Ok(None);
    }

    Ok(parse_proxy_protocol_v2_addresses(family, &addresses))
}

fn parse_proxy_protocol_v2_addresses(family: u8, addresses: &[u8]) -> Option<SocketAddr> {
    match family {
        0x11 if addresses.len() >= 12 => {
            let source = <[u8; 4]>::try_from(&addresses[0..4]).ok()?;
            let port = u16::from_be_bytes([addresses[8], addresses[9]]);

            Some(SocketAddr::from((std::net::Ipv4Addr::from(source), port)))
        }
        0x21 if addresses.len() >= 36 => {
            let source = <[u8; 16]>::try_from(&addresses[0..16]).ok()?;
            let port = u16::from_be_bytes([addresses[32], addresses[33]]);

            Some(SocketAddr::from((std::net::Ipv6Addr::from(source), port)))
        }
        _ => None,
    }
}

async fn read_proxy_protocol_v1(tcp: &mut TcpStream) -> std::io::Result<Option<SocketAddr>> {
    let mut seen = [0u8; PROXY_V1_PAYLOAD_MAX_LENGTH];

    let available = tcp.peek(&mut seen).await?;
    let Some(line_length) = seen[..available]
        .windows(2)
        .position(|pair| pair == b"\r\n")
    else {
        return Ok(None);
    };

    tcp.read_exact(&mut seen[..line_length + 2]).await?;

    Ok(parse_proxy_protocol_v1_line(&seen[..line_length]))
}

fn parse_proxy_protocol_v1_line(line: &[u8]) -> Option<SocketAddr> {
    let line = std::str::from_utf8(line).ok()?;
    let mut fields = line.split(' ');

    if fields.next() != Some("PROXY") {
        return None;
    }

    match fields.next() {
        Some("TCP4" | "TCP6") => {}
        _ => return None,
    }

    let source: IpAddr = fields.next()?.parse().ok()?;
    let _destination = fields.next()?;
    let port: u16 = fields.next()?.parse().ok()?;

    Some(SocketAddr::from((source, port)))
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::net::Ipv6Addr;
    use std::time::Duration;

    use tokio::io::AsyncWriteExt as _;
    use tokio::net::TcpListener;

    use super::*;
    use crate::testing::parse_each;

    // `client_from_proxy_protocol` must read only what a PROXY header holds, so the sender
    // keeps the connection open until it returns. The timeout stops a read that waits for more.
    async fn client_from_proxy_protocol_on(sent_bytes: &[u8]) -> (Option<SocketAddr>, Vec<u8>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut sender = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut connection, _) = listener.accept().await.unwrap();
        sender.write_all(sent_bytes).await.unwrap();

        let proxied_client = tokio::time::timeout(
            Duration::from_secs(5),
            client_from_proxy_protocol(&mut connection),
        )
        .await
        .expect("the read waited for bytes that were never sent")
        .unwrap();

        sender.shutdown().await.unwrap();
        let mut unread_bytes = Vec::new();
        connection.read_to_end(&mut unread_bytes).await.unwrap();

        (proxied_client, unread_bytes)
    }

    fn proxy_v2_header(family: u8, addresses: &[u8]) -> Vec<u8> {
        let mut header = PROXY_V2_SIGNATURE.to_vec();
        header.push(0x21);
        header.push(family);
        header.extend_from_slice(&u16::try_from(addresses.len()).unwrap().to_be_bytes());
        header.extend_from_slice(addresses);

        header
    }

    #[test]
    fn contains_matches_only_the_addresses_in_a_network() {
        let networks = Networks::new(&parse_each(&[
            "10.0.0.7",
            "192.168.1.0/24",
            "2001:db8::/32",
        ]));

        for address_inside in ["10.0.0.7", "192.168.1.9", "2001:db8::dead"] {
            assert!(
                networks.contains(address_inside.parse().unwrap()),
                "{address_inside}"
            );
        }

        for address_outside in ["10.0.0.8", "192.168.2.9", "2001:db9::1"] {
            assert!(
                !networks.contains(address_outside.parse().unwrap()),
                "{address_outside}"
            );
        }
    }

    #[test]
    fn contains_ipv4_mapped_address_or_network_matches_as_ipv4() {
        let networks = Networks::new(&parse_each(&[
            "10.0.0.0/8",
            "::ffff:172.16.0.0/108",
            "::ffff:192.168.1.1",
        ]));

        for address_inside in [
            "::ffff:10.0.0.7",
            "172.16.0.7",
            "::ffff:172.16.0.7",
            "192.168.1.1",
        ] {
            assert!(
                networks.contains(address_inside.parse().unwrap()),
                "{address_inside}"
            );
        }

        for address_outside in ["11.0.0.1", "::ffff:11.0.0.1", "2001:db8::1", "192.168.1.2"] {
            assert!(
                !networks.contains(address_outside.parse().unwrap()),
                "{address_outside}"
            );
        }
    }

    #[test]
    fn parse_mapped_network_returns_the_ipv4_network() {
        let parsed = "::ffff:10.0.0.0/104".parse::<NetworkAddress>().unwrap();

        assert_eq!(parsed, "10.0.0.0/8".parse::<NetworkAddress>().unwrap());
    }

    #[test]
    fn parse_mapped_network_with_prefix_below_96_returns_error() {
        for invalid_network in ["::ffff:10.0.0.0/8", "::ffff:10.0.0.0/95", "::ffff:0:0/0"] {
            let parsed = invalid_network.parse::<NetworkAddress>();

            assert_matches!(parsed, Err(_), "{invalid_network:?}");
        }
    }

    #[test]
    fn parse_network_without_usable_prefix_returns_error() {
        for invalid_network in ["10.0.0.0/33", "2001:db8::/129", "10.0.0.0/", "not/8", ""] {
            let parsed = invalid_network.parse::<NetworkAddress>();

            assert_matches!(parsed, Err(_), "{invalid_network:?}");
        }
    }

    #[tokio::test]
    async fn client_from_proxy_protocol_opening_shorter_than_signature_is_left_alone() {
        let (proxied_client, unread_bytes) = client_from_proxy_protocol_on(b"\x16\x03\x01").await;

        assert_eq!(proxied_client, None);
        assert_eq!(unread_bytes, b"\x16\x03\x01");
    }

    #[tokio::test]
    async fn client_from_proxy_protocol_v2_local_header_names_nobody_and_comes_off_the_stream() {
        let mut sent_bytes = PROXY_V2_SIGNATURE.to_vec();
        // Version 2, command LOCAL, and the AF_UNSPEC family the spec asks a LOCAL sender for.
        sent_bytes.push(0x20);
        sent_bytes.push(0x00);
        sent_bytes.extend_from_slice(&0u16.to_be_bytes());
        sent_bytes.extend_from_slice(b"\x16\x03\x01hello");

        let (proxied_client, unread_bytes) = client_from_proxy_protocol_on(&sent_bytes).await;

        assert_eq!(proxied_client, None);
        assert_eq!(unread_bytes, b"\x16\x03\x01hello");
    }

    #[tokio::test]
    async fn client_from_proxy_protocol_v2_header_of_wrong_version_is_refused_before_its_length_is_used()
     {
        let mut sent_bytes = PROXY_V2_SIGNATURE.to_vec();
        // Version 3, PROXY command, TCP over IPv4, announcing a block that never arrives.
        sent_bytes.push(0x31);
        sent_bytes.push(0x11);
        sent_bytes.extend_from_slice(&u16::MAX.to_be_bytes());

        let (proxied_client, _) = client_from_proxy_protocol_on(&sent_bytes).await;

        assert_eq!(proxied_client, None);
    }

    #[tokio::test]
    async fn client_from_proxy_protocol_v2_header_names_the_client_and_comes_off_the_stream() {
        let mut addresses = Vec::new();
        addresses.extend_from_slice(&[198, 51, 100, 1]);
        addresses.extend_from_slice(&[10, 0, 0, 7]);
        addresses.extend_from_slice(&4321u16.to_be_bytes());
        addresses.extend_from_slice(&443u16.to_be_bytes());

        let mut sent_bytes = proxy_v2_header(0x11, &addresses);
        sent_bytes.extend_from_slice(b"\x16\x03\x01hello");

        let (proxied_client, unread_bytes) = client_from_proxy_protocol_on(&sent_bytes).await;

        assert_eq!(proxied_client, Some("198.51.100.1:4321".parse().unwrap()));
        assert_eq!(
            unread_bytes, b"\x16\x03\x01hello",
            "the handshake reads what follows"
        );
    }

    #[tokio::test]
    async fn client_from_proxy_protocol_v2_header_carries_an_ipv6_client() {
        let mut addresses = Vec::new();
        addresses.extend_from_slice(&"2001:db8::1".parse::<Ipv6Addr>().unwrap().octets());
        addresses.extend_from_slice(&"2001:db8::7".parse::<Ipv6Addr>().unwrap().octets());
        addresses.extend_from_slice(&4321u16.to_be_bytes());
        addresses.extend_from_slice(&443u16.to_be_bytes());
        let sent_bytes = proxy_v2_header(0x21, &addresses);

        let (proxied_client, _) = client_from_proxy_protocol_on(&sent_bytes).await;

        assert_eq!(proxied_client, Some("[2001:db8::1]:4321".parse().unwrap()));
    }

    #[tokio::test]
    async fn client_from_proxy_protocol_v1_header_names_the_client_and_comes_off_the_stream() {
        let mut sent_bytes = b"PROXY TCP4 198.51.100.1 10.0.0.7 4321 443\r\n".to_vec();
        sent_bytes.extend_from_slice(b"\x16\x03\x01hello");

        let (proxied_client, unread_bytes) = client_from_proxy_protocol_on(&sent_bytes).await;

        assert_eq!(proxied_client, Some("198.51.100.1:4321".parse().unwrap()));
        assert_eq!(
            unread_bytes, b"\x16\x03\x01hello",
            "the handshake reads what follows"
        );
    }

    #[tokio::test]
    async fn client_from_proxy_protocol_v1_unknown_header_names_nobody_and_comes_off_the_stream() {
        let mut sent_bytes = b"PROXY UNKNOWN\r\n".to_vec();
        sent_bytes.extend_from_slice(b"\x16\x03\x01hello");

        let (proxied_client, unread_bytes) = client_from_proxy_protocol_on(&sent_bytes).await;

        assert_eq!(proxied_client, None);
        assert_eq!(unread_bytes, b"\x16\x03\x01hello");
    }

    #[tokio::test]
    async fn client_from_proxy_protocol_opening_with_a_handshake_is_left_alone() {
        let sent_bytes = b"\x16\x03\x01\x00\xff\x01\x00\x00\xfb\x03\x03 a client hello";

        let (proxied_client, unread_bytes) = client_from_proxy_protocol_on(sent_bytes).await;

        assert_eq!(proxied_client, None);
        assert_eq!(unread_bytes, sent_bytes, "the handshake reads what it sent");
    }
}
