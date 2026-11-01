# The Paron Tunnel Protocol (PTP), Version 1

## Abstract

This document specifies the Paron Tunnel Protocol (PTP), an application
protocol by which a host that cannot be addressed directly makes selected
local services reachable to a remote party.  The initiating party, termed
the agent, establishes and maintains a single outbound connection.  The
accepting party, termed the broker, opens byte streams over that connection
which terminate at services on the agent's host: TCP ports, UDP ports and
UNIX-domain sockets.

PTP also specifies a bootstrap procedure, termed joining, that binds an
asymmetric keypair to an agent identifier using a single-use credential.

PTP runs over TLS 1.3.  This document specifies one wire protocol and two
transport mappings, one over QUIC and one over TLS over TCP with stream
multiplexing.  The two mappings realise the identical protocol.  PTP
delegates every judgement to an implementation-supplied policy.  This
document specifies the requirements PTP places on the transport, on the
policy, and on the state machines of each role.

## Status of This Memo

This document specifies the PTP/1 protocol for the paron project.  It is
normative for implementations claiming PTP/1 conformance.

Requirements binding a separately implemented surface are individually
labelled and testable: transport ([TRN-n][s7], [Section 7][s7]), policy
(POL-n, [Section 8][s8]), state machine (STM-n, [Sections 4.3][s4.3] and
[9][s9]), versioning (VER-n, [Section 10][s10]) and conformance (CNF-n,
[Section 11][s11]).  An implementation claiming conformance cites the labels
it satisfies.  Requirements stated in [Sections 2][s2] and [3][s3] define
the encodings and the message vocabulary themselves; they are cited by
section number and bind every implementation.  Labelling requirements for
citation follows [RFC4787][rfc4787] and [RFC5382][rfc5382]; typing the
labels by category departs from those documents, which use a single flat
sequence.  The categories name what a requirement binds, because a transport
implementation, a policy and a protocol implementation are separately
supplied.

Labels of the form SEC-n ([Section 12.1][s12.1]) are not requirements.  They
state properties the protocol provides wherever the labelled requirements
are met; they are not separately claimed.  [Appendix D][appendix-d] lists
every label.

Sections 1 through 14 and Appendices [A][appendix-a] and [B][appendix-b] are
normative.  Appendices [C][appendix-c] and [D][appendix-d] are informative.

## Table of Contents

- [1. Introduction][s1]
    - [1.1. Requirements Language][s1.1]
    - [1.2. Terminology][s1.2]
    - [1.3. Protocol Surfaces][s1.3]
- [2. Encodings][s2]
    - [2.1. Identifiers and Integers][s2.1]
    - [2.2. Public Key Encoding][s2.2]
    - [2.3. Selector Encoding][s2.3]
    - [2.4. Control Stream and Frames][s2.4]
    - [2.5. Tunnel Streams][s2.5]
- [3. Message Vocabulary][s3]
    - [3.1. Messages][s3.1]
    - [3.2. Access Verdicts][s3.2]
- [4. Protocol State Machines][s4]
    - [4.1. Agent State Machine][s4.1]
    - [4.2. Broker State Machine][s4.2]
    - [4.3. Stream Sub-Protocol][s4.3]
- [5. Procedures][s5]
    - [5.1. Serving][s5.1]
    - [5.2. Joining][s5.2]
- [6. Connection Management][s6]
    - [6.1. Registration][s6.1]
    - [6.2. Liveness][s6.2]
    - [6.3. Transport Selection][s6.3]
- [7. Transport Requirements][s7]
- [8. Policy Requirements][s8]
    - [8.1. Broker Policy][s8.1]
    - [8.2. Agent Policy][s8.2]
- [9. State Machine Requirements][s9]
- [10. Versioning][s10]
    - [10.1. Version Rules][s10.1]
    - [10.2. Version-Invariant Rules][s10.2]
    - [10.3. Reserved Extensions][s10.3]
- [11. Conformance][s11]
- [12. Security Considerations][s12]
    - [12.1. Properties][s12.1]
    - [12.2. Threat Analysis][s12.2]
    - [12.3. The Consumer][s12.3]
    - [12.4. Fixed Identities][s12.4]
    - [12.5. Broker Locality][s12.5]
    - [12.6. Version Downgrade][s12.6]
    - [12.7. Mapping Downgrade][s12.7]
    - [12.8. Timing of Denials][s12.8]
    - [12.9. Unenumerated Announcements][s12.9]
- [13. IANA Considerations][s13]
- [14. References][s14]
    - [14.1. Normative References][s14.1]
    - [14.2. Informative References][s14.2]
- [Appendix A. QUIC Transport Mapping (Normative)][appendix-a]
- [Appendix B. TLS over TCP Transport Mapping (Normative)][appendix-b]
- [Appendix C. Design Rationale (Informative)][appendix-c]
    - [C.1. Two Verdicts][sC.1]
    - [C.2. Idempotent Joining][sC.2]
    - [C.3. Replacement Rather Than Rejection][sC.3]
    - [C.4. Liveness Parameters Are Local][sC.4]
    - [C.5. Absence of SHOULD][sC.5]
    - [C.6. One Wire Protocol, Two Mappings][sC.6]
    - [C.7. The Choice of yamux][sC.7]
    - [C.8. Datagrams over Streams][sC.8]
    - [C.9. A Stream Proxy, Not a Packet Tunnel][sC.9]
    - [C.10. Withdrawal as a Separate Operation][sC.10]
    - [C.11. Freezing Rather Than Refusing][sC.11]
    - [C.12. Bounding Admitted Connections][sC.12]
    - [C.13. Bounding Who Holds Admission][sC.13]
    - [C.14. Detection Without Overlap][sC.14]
    - [C.15. Fixed Identities][sC.15]
    - [C.16. The Trusted Consumer][sC.16]
    - [C.17. One Broker Process][sC.17]
    - [C.18. Reconnection Backoff][sC.18]
    - [C.19. Binary Claims][sC.19]
    - [C.20. Ranges over Ports, a Wildcard over Paths][sC.20]
    - [C.21. A Counter, Not a Clock][sC.21]
    - [C.22. Racing Connection Attempts][sC.22]
- [Appendix D. Summary of Labels (Informative)][appendix-d]
    - [D.1. Transport Requirements][sD.1]
    - [D.2. Policy Requirements][sD.2]
    - [D.3. State Machine Requirements][sD.3]
    - [D.4. Versioning Requirements][sD.4]
    - [D.5. Conformance Tests][sD.5]
    - [D.6. Security Properties][sD.6]

---

## 1. Introduction

An agent executes on a host that a broker cannot address: the host is behind
a network address translator, behind a firewall, or otherwise unroutable
from the broker's position.  The agent establishes one outbound connection
to the broker and maintains it.  Over that connection the broker, and only
the broker, opens streams; each stream terminates at a service on the
agent's own host, identified by a selector.  The direction of connection
establishment is inverted with respect to the direction of service.

An agent identifier is known to a broker before the agent's keypair is.
Joining is the procedure that binds the two, using a single-use credential
issued out of band.  Joining is an authentication procedure: the keypair a
broker binds is one whose possession was proven on the connection that
carried the credential.

PTP delegates judgement to a policy ([Section 8][s8]) and rests on TLS 1.3
for authentication, confidentiality and integrity ([Section 7][s7]).  PTP
itself specifies two state machines, one per role, the message vocabulary
they exchange, and the two transport mappings the messages travel over
([Appendix A][appendix-a], [Appendix B][appendix-b]).

### 1.1. Requirements Language

The key words "MUST", "MUST NOT", and "MAY" in this document are to be
interpreted as described in BCP 14 [RFC2119][rfc2119] [RFC8174][rfc8174]
when, and only when, they appear in all capitals.

This document does not use "SHOULD" or "RECOMMENDED".  Every requirement is
either mandatory or an explicit permission.  Rationale appears in [Appendix
C.5][sC.5].

### 1.2. Terminology

**Agent.**  The party that establishes the connection.  An agent serves
streams to services on its own host and to no other host.

**Broker.**  The party that accepts connections.  A broker cannot address an
agent's host directly; it admits agents and opens streams to them on behalf
of its consumers.

**Consumer.**  A party on whose behalf a broker opens streams.  A consumer
is within the broker's trust boundary: this document places no requirement
between a broker and its consumers ([Section 12.3][s12.3]).

**Connection.**  One mutually authenticated, multiplexed association between
one agent and one broker, established over a transport mapping.

**Transport mapping.**  A normative realisation of this protocol over a
concrete transport.  This document specifies two, in [Appendix
A][appendix-a] and [Appendix B][appendix-b].  Both realise the identical
protocol; above the transport, the mapping in use is invisible.

**Stream.**  One bidirectional byte pipe multiplexed over a connection.  The
control stream is one; every other stream is a tunnel stream, opened only by
the broker and addressed by a selector.

**Control stream.**  The stream on which every frame of [Section 2.4][s2.4]
travels.  A connection has exactly one, opened by the agent; each transport
mapping names which stream it is.

**Tunnel-stream preamble.**  The octets a tunnel stream opens with: a host
designator, then exactly one selector ([Section 2.5][s2.5]).  The broker
transmits it to name what the stream terminates at, and the agent answers it
with one octet.

**Selector.**  The value identifying either what a tunnel stream terminates
at or a set of such values.  PTP/1 defines six selector types ([Section
2.3][s2.3]).  The types `tcp-port`, `udp-port` and `unix-path` each name one
service, and only these three appear in a tunnel-stream preamble.  The types
`tcp-port-range`, `udp-port-range` and `unix-path-any` name no service and
appear only in an announced set.  Further selector types are reserved
([Section 10.3][s10.3]).

**Host designator.**  The value in an `ANNOUNCE`, and in a tunnel-stream
preamble, naming which host serves the selectors beside it.  PTP/1 defines
exactly one, denoting the agent's own host; further designators are reserved
in both places ([Section 10.3][s10.3]).  An agent serves selectors from its
own host and from no other ([SEC-5][sec-5]).

**Admission.**  A broker's transmission of `ACCESS`(Granted) on a
connection.  A connection is admitted from that transmission; the agent
observes admission on receipt of the frame.

**Announced set.**  The selectors most recently carried by an `ANNOUNCE` on
a connection ([Section 3.1][s3.1]).  The set is informational: a broker
records it for its operator ([STM-13][stm-13]) and derives nothing else from
it, and it places no obligation on the agent.  [POL-8][pol-8] is the agent's
authority at the moment a stream arrives, whatever the set contains.

**Agent identifier.**  A UUID [RFC9562][rfc9562], carried in its 16-octet
binary form ([Section 2.1][s2.1]).  An agent identifier is public data; it
appears in operator records and logs.

**Sequence.**  A strictly increasing integer an agent presents in every
claim, one per connection.  A broker accepts a claim only where its sequence
exceeds the last it accepted for that identifier; two parties holding one
keypair cannot both keep pace with one sequence, on which [SEC-6][sec-6]
rests.

**Frozen.**  An agent identifier whose keypair has been removed under
[POL-5][pol-5] because a sequence went backwards.  A frozen identifier is
indistinguishable from one that never joined, and every claim for it is
answered `ACCESS`(Denied), from every keypair, until an operator issues a
further join token by means outside this document.

**Agent identity.**  An agent identifier together with an asymmetric
keypair.  The public half is encoded per [Section 2.2][s2.2].  The private
half is confined to the host that generated it ([POL-9][pol-9]).

**Broker identity.**  An asymmetric keypair, termed the broker root key,
whose public half is encoded per [Section 2.2][s2.2].  An agent pins its
fingerprint.  The broker root key certifies broker serving keys and is not
presented for proof of possession in a handshake.  The identity is fixed for
the lifetime of a deployment ([Section 12.4][s12.4]).

**Broker serving key.**  An asymmetric keypair a broker proves possession of
during the handshake ([TRN-2][trn-2]), certified by the broker root key.  A
broker replaces a broker serving key without contacting any agent ([Section
12.4][s12.4]).

**Rollback index.**  An unsigned integer a broker certifies with a broker
serving key and never decreases ([TRN-13][trn-13]).  An agent records the
highest rollback index it has accepted and refuses a lower one
([POL-11][pol-11], [TRN-2][trn-2]), which is what ends the usefulness of a
broker serving key that has been replaced.

**Join token.**  A single-use, expiring credential of at least 128 bits of
entropy, issued by a broker's environment and delivered to an agent out of
band.  Its confidential part is transmitted in band at most once per
connection.

**State machine.**  The protocol logic of one role, specified in [Section
4][s4].  A state machine consults policy and drives the transport.  It makes
no judgements of its own ([STM-9][stm-9]).

**Registration.**  The broker-side association of an agent identifier with
the connection currently able to reach it, established at the grant of a
`CLAIM` ([Section 6.1][s6.1]).

**Policy.**  Implementation-supplied logic answering the judgements of
[Section 8][s8].

### 1.3. Protocol Surfaces

An implementation exposes three interfaces and implements one contract:

| Surface | Invoked by | Operations |
|---------|-----------|------------|
| Agent driving interface | the agent application | join, serve |
| Broker driving interface | the broker application | listen, identity |
| Broker consumer interface | a consumer of streams | open(agent, selector) |
| Wire contract | not invoked | [Section 4][s4] |

Each role implements the half of the [Section 4][s4] state machine belonging
to it.

## 2. Encodings

Every integer in this protocol is unsigned and is transmitted in network
byte order.  Every length is a count of octets.

### 2.1. Identifiers and Integers

An agent identifier is transmitted as the 16-octet binary form of
[RFC9562][rfc9562].  Operator-facing surfaces display the string
representation of that document; comparison is of the 16 octets.

A sequence is 8 octets.  The value 0 is not a sequence.

A protocol version is 1 octet.  PTP/1 is the value 0x01.  The value 0x00 is
not a version.

### 2.2. Public Key Encoding

Agent and broker public keys MUST be encoded as a DER-encoded
SubjectPublicKeyInfo structure, as defined in Section 4.1.2.7 of
[RFC5280][rfc5280].  Key comparison, wherever this document requires one, is
byte equality over that encoding.

A key travels inside an X.509 v3 certificate presented during the TLS
handshake ([TRN-2][trn-2], [TRN-3][trn-3]).  A certificate is a container for
the SubjectPublicKeyInfo, for the signature over it, and for the rollback
index of [TRN-13][trn-13] where this document requires one.  The verifying
party extracts those fields, verifies the signature where this document
requires one, and MUST ignore every other field, including validity, subject,
issuer and extensions.  A certificate over which this document requires no
signature verification is self-signed.

No party evaluates a validity period.  An agent whose clock is wrong, or
whose host has no clock at all, therefore connects on the same terms as any
other; the rollback index, and not a date, is what bounds the life of a key
([Appendix C.21][sC.21]).

The fingerprint of a public key is the string formed by a hash function name,
a colon, and the hexadecimal encoding of that function applied to the encoded
SubjectPublicKeyInfo.  An implementation MUST support the name `sha256`,
denoting SHA-256 [RFC6234][rfc6234].  Two fingerprints are equal when their
hash function names are equal and their decoded digests are byte-equal.  On
meeting a name it does not implement, an implementation MUST refuse the
fingerprint, and MUST NOT compare the remainder as octets.

A fingerprint is of one length for every key algorithm, whereas an encoded
SubjectPublicKeyInfo is not: its length follows the algorithm of the key it
encodes.  This document therefore pins a fingerprint wherever the pinned
value is transcribed out of band ([POL-7][pol-7]), and compares encoded keys
everywhere else.

The encoding is self-describing.  Introduction of a further key algorithm
therefore does not require a protocol version change.  Which algorithms are
usable at all is a property of the TLS implementation, which cannot prove
possession of a key it is unable to sign with; a policy MAY additionally
refuse one when [POL-2][pol-2] binds a key, that being the only point at
which it chooses.

### 2.3. Selector Encoding

A selector is encoded as a 1-octet selector type followed by a value whose
form the type fixes:

| Type | Name | Value |
|------|------|-------|
| 0x01 | `tcp-port` | port, 2 octets, not 0 |
| 0x02 | `udp-port` | port, 2 octets, not 0 |
| 0x03 | `unix-path` | length, 2 octets, 1 to 4096; then that many octets |
| 0x04 | `tcp-port-range` | low, 2 octets, not 0; high, 2 octets, not below low |
| 0x05 | `udp-port-range` | low, 2 octets, not 0; high, 2 octets, not below low |
| 0x06 | `unix-path-any` | none |

Every selector type either names one service on the designated host, or
names a set of selectors and no service.  A selector that names one service
may appear wherever a selector may.  A selector that names no service is
meaningful only in an announced set ([Section 3.1][s3.1]), and MUST NOT
appear in a tunnel-stream preamble ([Section 2.5][s2.5]).  Of the six types
above, `tcp-port`, `udp-port` and `unix-path` each name one service, and the
other three name no service.

A `tcp-port` identifies a TCP service on the designated host, and a
`udp-port` a UDP service.  The two port spaces are disjoint, so the two are
separate selector types rather than one type with a flag.  A `unix-path`
identifies a UNIX-domain socket by filesystem path; the octet 0x00 MUST NOT
appear in the path.

A `tcp-port-range` identifies every `tcp-port` whose port lies between the
range's low and high values inclusive, and a `udp-port-range` every
`udp-port` over the same span.  The pair 1 and 65535 identifies every port
of its type.  A `unix-path-any` identifies every `unix-path`.  Each names a
set an agent announces without enumerating the selectors themselves.

Two selectors are equal where their encodings are equal.  A selector that
names no service is therefore distinct from every selector it identifies,
and two such selectors identifying overlapping sets are distinct from each
other.  Selector types other than the six above are reserved ([Section
10.3][s10.3]).

### 2.4. Control Stream and Frames

The first octet an agent transmits on the control stream is the protocol
version ([TRN-11][trn-11]).  Every later octet on the control stream, in
both directions, belongs to a frame.  The broker transmits no version octet.

A frame is a 1-octet type, a 2-octet payload length, and a payload of that
many octets:

| Type | Message | Payload |
|------|---------|---------|
| 0x01 | `CLAIM` | agent identifier, 16 octets; sequence, 8 octets |
| 0x02 | `ACCESS` | verdict, 1 octet: 0x01 Granted, 0x02 Denied |
| 0x03 | `JOIN` | token, 1 to 4096 octets |
| 0x04 | `ANNOUNCE` | host designator, 1 octet; count, 2 octets; then count selector encodings |
| 0x05 | `PING` | none |
| 0x06 | `PONG` | none |

PTP/1 defines one host designator, 0x01, denoting the agent's own host.

`PING` and `PONG` realise the probes of [Section 6.2][s6.2].  Either role
MAY transmit `PING` in any state from LINKED onward; a role receiving `PING`
answers `PONG`.  Neither frame advances any state, and neither appears in
the tables of [Section 4][s4].

An `ANNOUNCE` carries a set: a selector MUST NOT appear more than once in
one `ANNOUNCE`.  A count of 0 is a valid encoding ([Section 3.1][s3.1]).  An
`ANNOUNCE` occupies one frame, so the announced set is bounded by the
frame's payload length; an agent MUST NOT announce a set whose encoding
exceeds one frame.

A frame is malformed where its type is not listed above, where its payload
disagrees with the grammar of its type, or where a field carries an excluded
value: a length other than 24 for `CLAIM`, a verdict octet other than 0x01
or 0x02, a sequence of 0, a port of 0, a path length of 0 or exceeding 4096,
a 0x00 octet in a path, a range whose high port is below its low port, a
duplicate selector, a host designator other than 0x01, or a count
disagreeing with the payload extent.  A malformed frame is a protocol error
([STM-3][stm-3]): the connection ends.

### 2.5. Tunnel Streams

The broker opens a tunnel stream by transmitting a host designator, 1 octet,
followed by exactly one selector encoding ([Section 2.3][s2.3]), as the
stream's first octets.  Those octets are termed the tunnel-stream preamble.
A broker MUST NOT open a tunnel stream naming a selector that names no
service.  PTP/1 defines one host designator, 0x01, denoting the agent's own
host; the octet is carried here so that a future version can name another
host at the point where a stream is asked for ([Section 10.3][s10.3]).  The
agent answers with 1 octet: 0x01 realises `STREAM_ACCEPT` and 0x02 realises
`STREAM_REFUSE`.  Following 0x02, each side closes the stream.

Following 0x01, the stream's contents depend on the selector type.  For
`tcp-port` and `unix-path`, the stream carries the bytes of the service's
conversation, unframed, in both directions.

For `udp-port`, the stream carries records, in both directions: a 2-octet
length followed by that many octets.  One record corresponds to one
datagram, in each direction; a record of length 0 corresponds to an empty
datagram.  Datagram boundaries are preserved and datagram contents are not
interpreted.  A relaying implementation MUST bound the records it buffers
for a stream and MAY discard records when that bound is reached; discard is
the behaviour a UDP service already tolerates.

A host designator other than 0x01, octets that do not decode as a selector,
a selector that names no service, and an answer octet other than the two
defined, are a protocol error of the stream alone: the role detecting one
closes the stream, the connection is unaffected, and a broker reports such a
stream under [STM-8][stm-8].  The malformed-frame rule of [Section
2.4][s2.4] binds the control stream only.

## 3. Message Vocabulary

### 3.1. Messages

Messages travel as frames on the control stream ([Section 2.4][s2.4]),
except the stream sub-protocol messages, which are realised by the tunnel
stream itself ([Section 2.5][s2.5]):

| Message | Direction | Meaning |
|---------|-----------|---------|
| `CLAIM` | agent to broker | claims an agent identifier with a sequence; the keypair is the one proven at establishment ([TRN-3][trn-3]) |
| `ACCESS` | broker to agent | one verdict ([Section 3.2][s3.2]), answering `CLAIM` or `JOIN` |
| `JOIN` | agent to broker | the confidential part of a join token |
| `ANNOUNCE` | agent to broker | the set identifying the selectors the agent expects to serve, and the host designator serving them |
| `STREAM_OPEN` | broker to agent | opens a tunnel stream; carries a host designator and one selector |
| `STREAM_ACCEPT` | agent to broker | the stream is connected to its service |
| `STREAM_REFUSE` | agent to broker | the stream terminates at nothing |
| `PING` | either | a liveness probe ([Section 6.2][s6.2]) |
| `PONG` | either | the answer to a `PING` |

An agent transmits at most one `CLAIM` and at most one `JOIN` per connection
([STM-10][stm-10]).

An `ANNOUNCE` carries a set of selectors, and each `ANNOUNCE` replaces the
set carried by the one before it.  The set identifies the selectors the
agent expects to serve: a selector naming a service stands for itself, and a
selector naming no service stands for the selectors it identifies ([Section
2.3][s2.3]).  An empty set is a valid announcement and announces that the
agent expects to serve nothing.  Two selectors in one set MAY identify
overlapping sets; the rule of [Section 2.4][s2.4] against a repeated
selector is equality of encodings and does not bound overlap.  An agent MAY
transmit `ANNOUNCE` at any time while SERVING ([Section 4.1][s4.1]); an
`ANNOUNCE` in any other state is a protocol error ([STM-3][stm-3]).  Serving
transmits one on the grant and one on each change of the set ([Section
5.1][s5.1]).

The announced set is informational.  A broker MUST make the most recent
announced set available to its operator ([STM-13][stm-13]), MUST NOT answer
an `ANNOUNCE`, and MUST NOT derive any other behaviour from one: the set
does not bound the selectors a stream may name, does not gate registration,
and obliges the agent to nothing.  Every tunnel stream is answered by
[POL-8][pol-8] whatever selector it names, whether or not the selector was
announced ([Section 4.3][s4.3]).  An announcement carrying a selector that
names no service obliges the agent no differently from one carrying none.

### 3.2. Access Verdicts

`ACCESS` answers every admission attempt, that is, both `CLAIM` and `JOIN`.
It carries exactly one of:

**Granted.**  A keypair is bound to the claimed agent identifier, and it is
the keypair whose possession was proven on this connection.

**Denied.**  Every other case, including: no such identifier; an identifier
known but with no keypair bound to it; an identifier not eligible for a
tunnel; a keypair other than the bound one; and a version outside the
broker's range.

An implementation MUST implement both verdicts.  An implementation MUST NOT
define a third verdict.  A verdict states a fact about broker-side records;
it does not prescribe transport behaviour.

`ACCESS`(Denied) MUST NOT distinguish among the cases it covers, by its
content, by its timing, or by the transport mechanism realising it
([SEC-7][sec-7]).  What follows the answer may differ: a connection denied
under [VER-2][ver-2] then ends, which discloses the broker's version range
and nothing of its records ([Section 12.1][s12.1]).  In particular, a broker
does not disclose whether an agent identifier is known to it, and does not
signal that a join token is wanted; an agent invoking [Section 5.2][s5.2]
transmits its token after a denial regardless ([Section 4.1][s4.1]).
Rationale appears in [Appendix C.1][sC.1].

## 4. Protocol State Machines

### 4.1. Agent State Machine

The agent is the initiating role.

| State | Event | Action | Next state |
|-------|-------|--------|-----------|
| IDLE | invoke join or serve | establish connection; verify the broker root key ([TRN-2][trn-2]) | LINKED |
| LINKED | — | transmit the version octet and `CLAIM` | CLAIMED |
| CLAIMED | `ACCESS`(Granted), serve procedure | send `ANNOUNCE` carrying the set the agent expects to serve | SERVING |
| CLAIMED | `ACCESS`(Granted), join procedure | release the connection | END |
| CLAIMED | `ACCESS`(Denied), join procedure | send `JOIN` | JOINING |
| CLAIMED | `ACCESS`(Denied), serve procedure | release the connection ([STM-6][stm-6]; [STM-14][stm-14] applies) | END |
| JOINING | `ACCESS`(Granted) | release the connection | END |
| JOINING | `ACCESS`(Denied) | release the connection ([STM-14][stm-14] applies) | END |
| SERVING | the set the agent expects to serve changed | send `ANNOUNCE` carrying the new set | SERVING |
| any | malformed frame, or a frame not listed for the state | end the connection ([STM-3][stm-3]) | END |
| any | connection ends | — | END |

Tunnel streams are not events of this table.  A stream may arrive in any
state and is answered by [POL-8][pol-8] ([STM-19][stm-19]).

A conforming agent makes at most two admission attempts on one connection:
one `CLAIM`, and one `JOIN` where the procedure is joining
([STM-10][stm-10]).

### 4.2. Broker State Machine

The broker is the responding role.  One instance exists per connection.

| State | Event | Action | Next state |
|-------|-------|--------|-----------|
| LINKED | version octet outside the supported range ([VER-2][ver-2]) | report ([VER-4][ver-4]); send `ACCESS`(Denied) | END |
| LINKED | `CLAIM` | retain the identifier and the proven key ([TRN-3][trn-3]); invoke [POL-1][pol-1] | see below |
| — | [POL-1][pol-1] returned Denied | send `ACCESS`(Denied) | CLAIMED |
| — | [POL-1][pol-1] returned Granted | invoke [POL-6][pol-6] with the claim's sequence | see below |
| — | [POL-6][pol-6] returned Current | invoke [POL-3][pol-3] | see below |
| — | [POL-6][pol-6] returned Stale | invoke [POL-5][pol-5]; report ([STM-16][stm-16]); send `ACCESS`(Denied) | CLAIMED |
| — | [POL-6][pol-6] returned Undetermined | send `ACCESS`(Denied) | CLAIMED |
| — | [POL-3][pol-3] returned a handle | retain the handle; send `ACCESS`(Granted) | SERVING |
| — | [POL-3][pol-3] returned failure | end the connection | END |
| CLAIMED | `JOIN` | invoke [POL-2][pol-2] | see below |
| — | [POL-2][pol-2] returned success | send `ACCESS`(Granted) | JOINED |
| — | [POL-2][pol-2] returned failure | send `ACCESS`(Denied) | END |
| SERVING | `ANNOUNCE` | record the announced set; report it ([STM-13][stm-13]) | SERVING |
| SERVING | consumer request | open tunnel stream ([Section 4.3][s4.3]) | SERVING |
| any | malformed frame, or a frame not listed for the state | end the connection ([STM-3][stm-3]) | END |
| any | connection ends | release the handle | END |

A broker begins in LINKED once the transport reports the connection
established.  It reaches CLAIMED only by denying a `CLAIM`, so a `JOIN` is
answered under the agent identifier and proven key of the `CLAIM` on that
connection; a `JOIN` in LINKED has no identifier to be answered under and is
a protocol error ([STM-3][stm-3]).  A second `CLAIM`, in any state, is
likewise a protocol error ([STM-10][stm-10]).

[POL-6][pol-6] is invoked only after [POL-1][pol-1] has granted.  An agent
identifier is public ([Section 1.2][s1.2]), so a broker that checked the
sequence first would let anyone who knows an identifier freeze it by
presenting a low sequence with any keypair at all.  Requiring proof of
possession first ([TRN-3][trn-3]) confines the check to parties that already
hold the key.

A broker that is answered Stale MUST invoke [POL-5][pol-5] for the
identifier rather than refuse only the claim that went backwards.  Refusing
the stale claimant alone hands the identifier to whichever party connected
most recently, and in the case this exists to catch that is the thief
([Appendix C.11][sC.11]).  Freezing an identifier is [POL-5][pol-5] applied
to it ([Section 1.2][s1.2]): no state beyond the binding is required, and
expressing the freeze this way is what carries it across a broker restart,
since a keypair no longer bound is answered `ACCESS`(Denied) by
[POL-1][pol-1] without reference to any sequence.  A frozen identifier is
answered `ACCESS`(Denied) indistinguishably from any other denial
([SEC-7][sec-7]); that it is frozen is reported to the operator
([STM-16][stm-16]) and to nobody else.

A broker that is answered Undetermined MUST NOT invoke [POL-5][pol-5].  The
claim is refused, and nothing has been established about the sequence it
presented.

Where [POL-5][pol-5] invoked in answer to Stale returns failure, the keypair
remains bound and the next claim exceeding the recorded sequence is answered
Current and admitted.  No further recovery is available to the broker, the
binding not being the broker's to remove, so the failure is reported to the
operator ([STM-21][stm-21]).

A broker registers a connection when it grants a `CLAIM`: [POL-3][pol-3] is
invoked before that `ACCESS`(Granted) is transmitted, and the connection is
registered from the transmission ([Section 6.1][s6.1]).  A grant answering a
`JOIN` establishes no registration: the joining procedure releases the
connection on receiving it ([Section 5.2][s5.2]), and JOINED awaits that
release, every frame there being a protocol error ([STM-3][stm-3]).  A
[POL-3][pol-3] failure is a fault of the broker's environment, not a
verdict: the broker ends the connection, and no message reports the failure
to the agent.

Denial of a `CLAIM` does not end the connection: a denied agent invoking
[Section 5.2][s5.2] proceeds to `JOIN` on the same connection.  Denial of a
`JOIN` ends it; nothing further is permitted on that connection.  Every
`ACCESS`(Denied) is transmitted under the delay of [TRN-10][trn-10].

### 4.3. Stream Sub-Protocol

Tunnel streams are opened on admitted connections.  Each stream is an
independent exchange:

1.  The broker opens a stream carrying one selector ([Section 2.5][s2.5]).
2.  The agent invokes [POL-8][pol-8] with that selector.
3.  If [POL-8][pol-8] returns no target, the agent sends `STREAM_REFUSE`.
4.  If [POL-8][pol-8] returns a target, the agent establishes a connection
    to that target.  On success it sends `STREAM_ACCEPT` and then relays
    between the stream and that connection per [Section 2.5][s2.5].  On
    failure it sends `STREAM_REFUSE`.

<a id="stm-19"></a>**STM-19 (Stream admission).**  A broker MUST NOT open a
tunnel stream on a connection it has not admitted.  An agent answers every
tunnel stream by [POL-8][pol-8], in every state: streams and the control
stream are not mutually ordered ([TRN-7][trn-7]), so a stream may reach an
agent before the `ACCESS`(Granted) that admitted the connection, and such a
stream is not evidence of a non-conforming broker.  The announced set does
not bound the selector a stream may name ([Section 3.1][s3.1]); a stream
naming an unannounced selector is answered by [POL-8][pol-8] like any other.
Tunnel streams advance no state of [Section 4][s4] and appear in neither of
its tables.

<a id="stm-20"></a>**STM-20 (Accept ordering).**  An agent MUST NOT send
`STREAM_ACCEPT` before the connection to the target returned by
[POL-8][pol-8] is established.  `STREAM_REFUSE` therefore answers both a
selector [POL-8][pol-8] has no target for and a service that cannot be
reached, and the two are not distinguished ([SEC-7][sec-7]).

Streams MUST be concurrent ([TRN-6][trn-6]).  Relaying one stream MUST NOT
block the relaying of another.

## 5. Procedures

### 5.1. Serving

Serving advances the agent state machine from IDLE to SERVING and maintains
it there until the connection ends.  The procedure ends with the connection;
a caller re-invokes it to reconnect, subject to [STM-14][stm-14].

On reaching SERVING the agent transmits an `ANNOUNCE` carrying the set it
expects to serve, and transmits a further `ANNOUNCE` whenever that set
changes ([Section 4.1][s4.1]).  The announced set is informational; what the
agent serves is answered by [POL-8][pol-8], stream by stream ([Section
4.3][s4.3]).

An agent MUST treat `ACCESS`(Denied) during serving as a failure.  An agent
MUST NOT transmit a join token during serving ([STM-6][stm-6]).  Serving
does not bind a keypair; an agent whose keypair is not bound is denied, and
joining ([Section 5.2][s5.2]) is the only procedure that binds one.

### 5.2. Joining

Joining advances the agent state machine from IDLE to receipt of
`ACCESS`(Granted) and then releases the connection ([Section 4.1][s4.1]).

The postcondition of joining is that the agent's keypair is bound to the
agent identifier.  Where a broker answers `ACCESS`(Granted) to the `CLAIM`
of an agent invoking this procedure, the postcondition already holds.  In
this case the agent MUST treat the procedure as successful.  In this case
the agent MUST NOT transmit the join token ([STM-7][stm-7]).

A broker MUST commit the binding before transmitting `ACCESS`(Granted)
([POL-2][pol-2], [SEC-3][sec-3]).

An agent presenting a keypair other than the bound one receives
`ACCESS`(Denied), whether or not it also holds a valid join token.  A bound
keypair is replaced only by withdrawal ([POL-5][pol-5]) followed by a
further joining procedure.  An agent that has lost its private key, and an
agent whose private key is known to another party, are recovered this way
and by no other means this document defines.

## 6. Connection Management

### 6.1. Registration

A broker maintains at most one registration per agent identifier
([STM-11][stm-11]).  A registration is established by [POL-3][pol-3] at the
grant of a `CLAIM`: the broker invokes [POL-3][pol-3] before transmitting
the `ACCESS`(Granted) that answers a `CLAIM`, and the connection is
registered from that transmission.  Establishing a registration replaces any
registration held for the same identifier, and the broker ends the
connection the replaced registration was held by ([STM-11][stm-11]).

A grant answering a `JOIN` establishes no registration; the joining
procedure releases the connection on receiving it ([Section 5.2][s5.2]), and
no keypair was bound before the join, so no registration for the identifier
can predate it.  Registering such a connection would advertise, on every
join, a reachability the procedure ends a moment later, and a consumer
asking inside that moment would be handed a connection already closing.  An
agent that invokes the joining procedure while its keypair is already bound
is granted on its `CLAIM` ([STM-7][stm-7]), registers, and thereby replaces
the registration of a connection of its that is serving; the serving
procedure observes its connection end and reconnects ([STM-14][stm-14]).

A broker MUST make replacement visible to its operator ([STM-12][stm-12],
[SEC-6][sec-6]).

An agent is reachable through the consumer interface for exactly the
lifetime of its registration.  Authorisation of a consumer is outside the
scope of this document ([Section 12.3][s12.3]).  The consumer interface
reports one of the following.

| Condition | Reported |
|-----------|----------|
| No registration exists | No connection |
| The agent answered `STREAM_REFUSE`, or the open exceeded the bound of [TRN-6][trn-6] | Refusal |
| The agent answered `STREAM_ACCEPT` | The stream |
| Every other case | A transport or environment failure ([STM-8][stm-8]) |

### 6.2. Liveness

Two parameters govern liveness at each role: the probe interval, denoted K,
and the number of tolerated unanswered probes, denoted M.  A probe is a
`PING` frame on the control stream ([Section 2.4][s2.4]); its answer is a
`PONG` frame.

Both roles MUST probe.  A role MUST answer a peer's probe with `PONG`,
subject to the rate bound of [TRN-10][trn-10].  A role MUST NOT transmit
probes more often than its own K.  A role MAY treat a peer that transmits
probes more often than the floor on K below as having ended the connection;
a peer's own K is never known to the role, so no tighter judgement is
available to it, and the floor binds a configuration while the receiving
side is where an adversary is reached.

An implementation MUST determine liveness from received messages only.
Successful transmission MUST NOT be treated as evidence of liveness.  A role
counts consecutive probes of its own that no `PONG` has answered; where the
count exceeds M, the role MUST treat the connection as ended.

Each role chooses its own K and M.  Neither is exchanged and neither needs
to agree between roles: a role probes and its peer answers, so what a role
concludes rests on its own probing, and no assumption about the peer's
values enters it.  Rationale appears in [Appendix C.4][sC.4].

Outside a conformance mode ([TRN-8][trn-8]), K MUST NOT be shorter than 5
seconds.  An implementation MUST reject a configuration violating that bound
at startup rather than on the connection that first suffers from it.

An implementation MAY provide a conformance mode in which that floor does
not apply ([TRN-8][trn-8]).  The mode MUST NOT be the default.  An
implementation MUST make selection of the mode evident to an operator.  A
conformance run is the only setting in which both roles are under one
tester's control, and it is not a condition an implementation can infer for
itself.

K MUST NOT exceed 300 seconds.  M MUST NOT exceed 10.  The detection bound
(M+1)*K is how long a broker may offer a consumer an agent that is gone, so
it is not a matter for one role alone however locally its parameters are
chosen.

An implementation with no reason to choose otherwise uses K = 15 seconds and
M = 3.

A peer that has ceased answering is detected within (M+1)*K.  A peer that
sends nothing at all is the same case, probing beginning with the connection
rather than with any later state, so this document defines no idle timeout
beside the probe count and needs none ([Appendix C.4][sC.4]).

### 6.3. Transport Selection

An implementation states which transport mappings of this document it
implements.  A broker MAY listen on both at once.

An agent holds an ordered list of the mappings it is configured to use.  A
connection attempt is one mapping applied to one address of the broker.  The
connection attempts an agent starts to obtain one connection are termed a
race.  An agent orders the connection attempts of a race by mapping, in the
order of the list, and within one mapping by address, alternating address
families where the broker has addresses of more than one family
([RFC8305][rfc8305]).  A failure to establish is a connection attempt that
ends before [TRN-2][trn-2] completes.  A race in which every connection
attempt fails to establish is termed failed.

An agent continues the protocol on exactly one connection attempt of a race,
one that completed [TRN-2][trn-2], termed the selected attempt.  An agent
starts the connection attempts of a race in that order.  Until it selects a
connection attempt, an agent MUST start each later connection attempt of the
race when the one before it fails to establish or 250 milliseconds after the
one before it started, whichever is earlier.  A connection attempt in
progress continues when a later one starts.

On selecting a connection attempt, the agent MUST end every other connection
attempt of that race.  A connection ended under this rule is termed
overtaken.  An agent MUST NOT transmit a protocol message on any connection
attempt other than the selected one.

An `ACCESS`(Denied) is not a failure to establish; it is answered under
[Section 4.1][s4.1].  On receipt of `ACCESS`(Denied), an agent MUST NOT
include another mapping in a later race of the same invocation of a
procedure.  Rationale appears in [Appendix C.22][sC.22].

The two mappings realise the identical protocol, so a party that blocks one
obtains nothing but the other's transport properties ([Section
12.7][s12.7]).  An implementation with no reason to choose otherwise orders
[Appendix A][appendix-a] before [Appendix B][appendix-b].

## 7. Transport Requirements

The transport layer of a connection is TLS 1.3 over one of the mappings of
[Appendix A][appendix-a] and [Appendix B][appendix-b].  The requirements of
this section bind both mappings; each appendix states how its transport
discharges them.

<a id="trn-1"></a>**TRN-1 (Establishment).**  A transport implementation
provides connection establishment for the agent role and listening for the
broker role.  A listening operation takes an address, reports the address
actually bound, and ceases accepting when released.

<a id="trn-2"></a>**TRN-2 (Broker authentication).**  A connection uses TLS
1.3 [RFC8446][rfc8446] or later.  The broker presents a chain of exactly two
certificates: one carrying the broker serving key, followed by one carrying
the broker root key ([Section 2.2][s2.2]).

During the handshake, before transmitting its own certificate, the agent
performs three verifications and no others.  The agent MUST verify that the
fingerprint of the broker root key equals the pin ([POL-7][pol-7]).  The
agent MUST verify that the broker root key signed the certificate carrying
the broker serving key.  The agent MUST verify that the rollback index of
that certificate is not lower than the highest rollback index
[POL-11][pol-11] reports.  On failure of any of the three verifications the
agent MUST abort the handshake.  An agent transmits no protocol message
before all three verifications succeed.

The verifications are performed in the order stated.  A certificate the
broker root key did not sign is refused for that reason alone, no rollback
index being read from a certificate whose contents nothing vouches for.

On accepting a rollback index higher than the one [POL-11][pol-11] reports,
the agent MUST record it through [POL-11][pol-11] before transmitting any
protocol message, and MUST abort the handshake where recording fails.  An
agent that connected without recording would accept the superseded
certificate again on its next connection.

An agent MUST refuse a chain whose length is other than two, and MUST verify
no signature other than the one above.  Beyond the two SubjectPublicKeyInfo
values, that signature and the rollback index, an agent MUST NOT evaluate any
field of either certificate.  An agent MUST NOT refuse either certificate on
account of its validity period.  A certificate carrying a broker root key
MUST encode the notAfter value 99991231235959Z, which Section 4.1.2.5 of
[RFC5280][rfc5280] specifies for a certificate with no well-defined
expiration date.  Rationale appears in [Appendix C.15][sC.15] and [Appendix
C.21][sC.21].

<a id="trn-3"></a>**TRN-3 (Agent key proof).**  The broker MUST request a
client certificate, and a handshake presenting none fails.  The agent
presents a certificate whose SubjectPublicKeyInfo is the public half of its
identity ([POL-9][pol-9]).  The broker MUST complete the handshake whatever
certificate is presented, extracting the SubjectPublicKeyInfo and evaluating
nothing else; the significance of the key is determined by policy
([POL-1][pol-1]), and a broker that rejected a certificate at the TLS layer
would distinguish an unknown key from a known one by the layer that answers
([SEC-7][sec-7]).  The handshake's CertificateVerify proves possession of
the corresponding private key in a manner bound to this connection and
therefore neither replayable nor forwardable.

<a id="trn-4"></a>**TRN-4 (Confidentiality and integrity).**  All frames and
all stream data are confidentiality- and integrity-protected by the TLS
connection.  Session resumption MUST NOT be offered or accepted, a broker
MUST NOT issue session tickets, pre-shared-key modes MUST NOT be used, and
early data MUST NOT be sent or accepted: a handshake without
CertificateVerify from both parties satisfies neither [TRN-2][trn-2] nor
[TRN-3][trn-3].

<a id="trn-5"></a>**TRN-5 (Credential handling).**  The payload of a `JOIN`
frame MUST NOT be written to logs or to any record surviving the connection,
on either side, other than the commitment of [POL-2][pol-2].

<a id="trn-6"></a>**TRN-6 (Streams).**  Following admission, the broker side
is able to open independent bidirectional tunnel streams.  Streams MUST be
concurrent.  Streams MUST be subject to per-stream flow control.

A transport implementation MUST bound the time a tunnel stream open may go
unanswered, and MUST report a stream open that exceeds that bound to the
protocol as refused ([Section 4.3][s4.3]).  An agent MUST likewise bound its
own connection to the target [POL-8][pol-8] returned.  Neither bound is
fixed here: each constrains one role, and nothing breaks if two deployments
choose differently.  A stream open no answer is coming for is
indistinguishable from a slow one, and giving up early discloses nothing:
[Section 4.3][s4.3] already makes a refusal indistinguishable from a service
that is down ([SEC-7][sec-7]).

<a id="trn-7"></a>**TRN-7 (Ordering).**  The control stream delivers its
octets reliably and in order within each direction.  Within one tunnel
stream, bytes are delivered in the order they were written.  Across streams,
and between the control stream and the opening of tunnel streams, no
ordering holds at all, and an implementation MUST NOT depend on one.

<a id="trn-8"></a>**TRN-8 (Liveness).**  The probes of [Section 6.2][s6.2]
are frames, so probing belongs to the protocol implementation, using the
parameters of the role it runs.  A transport implementation MUST deliver
frames for as long as the connection lives, MUST report the ending of a
connection to the state machine, and MUST NOT impose an idle timeout of its
own shorter than the role's detection bound (M+1)*K.  It MUST accept the
shorter parameters a conformance mode supplies, including a K below the
floor of [Section 6.2][s6.2].

<a id="trn-9"></a>**TRN-9 (Default deny).**  A transport implementation MUST
refuse, or leave unnegotiated, every transport capability not required by
the version of this protocol its mapping realises: streams beyond those this
document names, unidirectional streams, datagram extensions, and any
capability the mapping's appendix lists.  Reliance on library defaults does
not satisfy this requirement.

<a id="trn-10"></a>**TRN-10 (Limits).**  A broker MUST NOT transmit
`ACCESS`(Denied) earlier than 1 second after receipt of the frame, or of the
version octet ([VER-2][ver-2]), that it answers.  The moment is fixed at
receipt, before any policy is invoked.  A delay added after a policy returns
does not satisfy this: such a delay carries the cost of the work done before
it, and the cases of [Section 3.2][s3.2] do not cost the same
([SEC-7][sec-7], [Section 12.8][s12.8]).

A broker MUST bound per-connection resource consumption, including the rate
at which frames from an admitted connection are processed.  Admission ends
the bounds of the paragraphs below, and a party holding a bound keypair
reaches SERVING and stays there; what it does on the connection it holds is
bounded here or nowhere ([Appendix C.12][sC.12]).

A broker MUST bound the number of connections it holds that have not been
admitted, and MUST refuse further connections while that bound is reached.
A broker MUST bound the time a connection may remain unadmitted, and MUST
end a connection that exceeds that bound.  That bound is separate from
[Section 6.2][s6.2]: liveness tolerates long silences from an established
peer, and a peer that has proven nothing is not established.  Neither value
is fixed here; each constrains one role only.

A broker MUST NOT permit connections originating from one source to occupy
the whole of the unadmitted-connection bound.  A broker MAY satisfy this
requirement by refusing with a probability that rises as the bound is
approached, by accounting per source, or by both.  Where its transport
validates a peer's address before committing connection state, a broker MUST
use that mechanism in addition ([Appendix A][appendix-a]).  Address
validation bounds what a spoofed source can cost and does not bound what a
party using addresses it holds can occupy, so it replaces neither of the two
mechanisms above.  Rationale appears in [Appendix C.13][sC.13].

<a id="trn-11"></a>**TRN-11 (Version).**  The first octet an agent transmits
on the control stream is the protocol version ([Section 2.1][s2.1]).  A
transport implementation MUST deliver it before any frame.

<a id="trn-12"></a>**TRN-12 (Demultiplexing).**  A PTP connection MUST
negotiate the TLS application-layer protocol identifier "ptp1"
[RFC7301][rfc7301].  A connection that does not negotiate it is not a PTP
connection and is outside this document.  A broker MAY share its listening
address with services that are not PTP, and MAY select the certificate it
presents by the identifier the peer offers, presenting the chain of
[TRN-2][trn-2] on PTP connections and any other certificate elsewhere.

<a id="trn-13"></a>**TRN-13 (Serving key index).**  The certificate carrying
a broker serving key MUST encode a rollback index as its X.509 serialNumber,
of at most eight octets.  A broker MUST NOT certify a broker serving key with
a rollback index lower than one it has certified before.  A broker MUST NOT
derive a rollback index from a clock.

A broker that cannot determine the highest rollback index it has certified
MUST NOT serve.  Certifying from a lower index would serve certificates every
agent that has connected refuses, and the broker would be unreachable by its
own fleet.

An agent MUST refuse a certificate carrying a broker serving key whose
serialNumber is wider than eight octets, leading zero octets excluded.  Eight
octets is the whole of the range this requirement permits, so a wider value
is not a rollback index and no comparison is defined over it.  Rationale
appears in [Appendix C.21][sC.21].

## 8. Policy Requirements

Policy answers the judgements this section labels.  Matters not listed here,
including the eligibility of an identifier, the storage of records, the
distribution of join tokens, the authorisation of consumers, and the
circumstances under which an operator invokes [POL-4][pol-4] or
[POL-5][pol-5], are outside the scope of this document.

### 8.1. Broker Policy

<a id="pol-1"></a>**POL-1 (Access).**  Given an agent identifier and a
public key whose possession has been proven, returns one verdict of [Section
3.2][s3.2].  This operation MUST NOT have side effects.  Key comparison is
byte equality over the encoding of [Section 2.2][s2.2].  Constant-time
comparison is not required; the compared values are public.

<a id="pol-2"></a>**POL-2 (Join).**  Given an agent identifier, a proven
public key, and a join token, binds the key to the identifier if the token
authorises it.  An implementation MUST verify the token in constant time
against a stored commitment.  An implementation MUST compute that commitment
using HMAC-SHA-256 [RFC2104][rfc2104] or a stronger construction, keyed by
material not derivable from the record store.  An implementation MUST NOT
store the token itself.  An implementation MUST verify that the token has
not expired.  An implementation MUST verify that the token was issued for
the agent identifier supplied.  An implementation MUST fail where a keypair
is already bound to that identifier.  An implementation MUST invalidate the
token on success.  An implementation MUST commit the binding before
returning success.

An implementation MUST perform verification, invalidation and binding as one
atomic operation, such that concurrent invocations presenting the same token
result in at most one binding ([SEC-4][sec-4]).

<a id="pol-3"></a>**POL-3 (Register).**  Given an agent identifier,
establishes a registration per [Section 6.1][s6.1] and returns either a
handle whose release ends the registration, or failure.  Establishing a
registration replaces any registration held for the same identifier
([STM-11][stm-11]).  This operation makes no judgement: failure is a fault
of the broker's environment, the broker answering it by ending the
connection ([Section 4.2][s4.2]), and no message reports it to the agent.

<a id="pol-4"></a>**POL-4 (Issuance).**  Join tokens MUST be generated with
at least 128 bits of entropy from a cryptographically secure pseudorandom
number generator.  A join token MUST carry an expiry.  A join token MUST be
stored only as a commitment satisfying [POL-2][pol-2].  A token is issued
for exactly one agent identifier.  Issuance MUST NOT withdraw an existing
binding; withdrawal is [POL-5][pol-5].

<a id="pol-5"></a>**POL-5 (Withdrawal).**  Given an agent identifier,
removes the keypair bound to that identifier, ends any registration for it,
and returns either success or failure.  Following withdrawal no keypair is
bound to the identifier, a claim presenting the withdrawn keypair is
answered `ACCESS`(Denied), and a join token issued for the identifier binds
the next keypair proven under [POL-2][pol-2].  Withdrawal of an identifier
with no keypair bound to it changes nothing and returns success.  Failure is
a fault of the broker's environment and not a verdict: the keypair remains
bound.  Rationale appears in [Appendix C.10][sC.10].

A broker invokes withdrawal on its operator's instruction, by means this
document does not specify, and where [POL-6][pol-6] answers Stale ([Section
4.2][s4.2]).  No message of this protocol requests withdrawal.  An agent can
therefore cause the withdrawal of an identifier, but only one whose bound
keypair the agent has already proven possession of, since [POL-6][pol-6] is
reached only after [POL-1][pol-1] has granted.

Withdrawal is stated here rather than left unmentioned because [Section
5.2][s5.2] admits no other way to replace a bound keypair, so an
implementation that omits it has no answer to a private key that was lost or
copied ([Section 12.2][s12.2]).

<a id="pol-6"></a>**POL-6 (Continuity).**  Given an agent identifier and the
sequence its claim presented, returns Current and records the sequence where
it exceeds the last recorded for that identifier, returns Stale where it
does not exceed it, and returns Undetermined where the record can be neither
read nor written.  Where no sequence has been recorded, any sequence is
Current.  Recording and comparison MUST be one atomic step
([STM-18][stm-18]).  Without atomicity, two connections racing may both be
told Current.

An implementation MUST distinguish Stale from Undetermined.  Both refuse the
claim and only the first freezes ([Section 4.2][s4.2]); a record that cannot
be answered is a fault of the broker's, and answering Stale on it would let
an interruption of the broker's own storage cost an agent its binding.

The record MAY be held only for as long as the broker runs.  A broker
restart ends every connection, so each agent still holding one reconnects
and establishes a record afresh, including an agent holding a copied
keypair: it restores the number the other holder will collide with,
unprompted.  What a restart discards is a record left by a holder that has
since disconnected, which is the exclusion [SEC-6][sec-6] states.

Binding a keypair under [POL-2][pol-2] clears the recorded sequence for that
identifier.  Without this a frozen identifier could not be recovered: a
holder that presented a sequence near the largest one representable leaves
no value an honest agent could exceed, and the identifier would be lost
rather than frozen.  Clearing costs nothing an attacker can use, since
[POL-2][pol-2] requires a token and only an operator issues one.

### 8.2. Agent Policy

<a id="pol-7"></a>**POL-7 (Broker pin).**  The fingerprint of the broker
root key to require ([Section 2.2][s2.2]), obtained out of band with the join
token.  Consumed by [TRN-2][trn-2].

<a id="pol-8"></a>**POL-8 (Target).**  Given a selector, returns either the
target a stream for that selector terminates at, or nothing.  This operation
is the agent's sole authority over what a stream reaches ([SEC-5][sec-5]);
the announced set does not constrain its answer ([Section 3.1][s3.1]).
Under PTP/1 a target MUST identify a service on the agent's own host: a TCP
endpoint for `tcp-port`, a UDP endpoint for `udp-port`, a socket path for
`unix-path`.  A selector that names no service is never presented to this
operation, [Section 2.5][s2.5] excluding one from a tunnel-stream preamble.

<a id="pol-9"></a>**POL-9 (Agent identity).**  The agent identifier and
keypair.  The identifier is public.  The private key MUST NOT leave the
host.

<a id="pol-10"></a>**POL-10 (Sequence).**  Returns the sequence for a
connection about to be established, strictly greater than any it has
returned before, and durably recorded before it is returned
([STM-15][stm-15]).  An implementation that cannot return such a sequence
MUST fail rather than return one.  An agent that has no such sequence MUST
NOT connect.

<a id="pol-11"></a>**POL-11 (Rollback index).**  Reports the highest rollback
index this agent has accepted under the pin of [POL-7][pol-7], and records a
higher one durably.  A record never lowers the index: where a higher index
is recorded, recording a lower one leaves the record unchanged.  The
connection attempts of one race ([Section 6.3][s6.3]) can complete
[TRN-2][trn-2] at the same time, each having read the index before the
other recorded.  An agent that has accepted none reports zero.  An
implementation that cannot read the recorded index MUST fail rather than
report zero, reporting zero being the rollback [TRN-2][trn-2] refuses.  A
change of [POL-7][pol-7] discards what was recorded, a rollback index
counting under one broker root key and meaning nothing under another.

[Section 2.1][s2.1] fixes the sequence at 64 bits, so a counter reaches its
last value only by a fault, and the obvious behaviour there is to wrap,
which is the one thing [STM-15][stm-15] forbids outright.  Refusing to
connect costs the agent its tunnel until an operator intervenes; wrapping
costs it the binding, since the broker answers a repeated sequence by
freezing ([Section 4.2][s4.2]).

## 9. State Machine Requirements

<a id="stm-1"></a>**STM-1 (Single driver).**  On the agent side, the
connection is driven by the agent state machine alone.

<a id="stm-2"></a>**STM-2 (Responder state).**  The broker state machine's
per-connection state is explicit.  A responding role cannot determine the
order of events.

<a id="stm-3"></a>**STM-3 (Protocol errors).**  A malformed frame ([Section
2.4][s2.4]), and any combination of state and frame not enumerated in
[Section 4.1][s4.1] or [Section 4.2][s4.2], is a protocol error.  A role
detecting one MUST end the connection.  A protocol error is not an admission
attempt and is not answered by any message of [Section 3][s3].  `PING` and
`PONG` are permitted in every state from LINKED onward, advance no state,
and are answered per [Section 6.2][s6.2]; the enumeration of this paragraph
does not reach them.

<a id="stm-4"></a>**STM-4 (Stream availability).**  The consumer interface
MUST NOT expose a connection before the broker has registered it ([Section
6.1][s6.1]).  A broker opens tunnel streams only on a connection it has
admitted, and an agent answers every tunnel stream by [POL-8][pol-8]
([STM-19][stm-19]).

<a id="stm-5"></a>**STM-5 (Release).**  Connections and registrations end by
release of the handle that owns them.

<a id="stm-6"></a>**STM-6 (Token confinement).**  A join token is used only
within the joining procedure and MUST NOT be retained after it.

<a id="stm-7"></a>**STM-7 (Join idempotence).**  Joining an agent whose
keypair is already bound MUST succeed without transmission of a join token
([Section 5.2][s5.2]).

<a id="stm-8"></a>**STM-8 (Error classification).**  An implementation MUST
distinguish refusal by policy, absence of a connection, and transport or
environment failure.

<a id="stm-9"></a>**STM-9 (No embedded judgement).**  State machines and
transport implementations MUST NOT make judgements reserved to policy by
[Section 8][s8].

<a id="stm-10"></a>**STM-10 (One claim per connection).**  An agent
transmits at most one `CLAIM` and at most one `JOIN` per connection, and
exactly one protocol version, as the control stream's first octet
([TRN-11][trn-11]).  A broker MUST end a connection carrying a second
`CLAIM` or a second `JOIN` ([STM-3][stm-3]).  An agent MUST NOT change
version in response to a transport failure ([VER-3][ver-3]).

<a id="stm-11"></a>**STM-11 (Admission replacement).**  A broker maintains
at most one registration per agent identifier.  The registration established
at admission MUST replace any existing registration for the same identifier,
and the broker MUST end the connection underlying the replaced registration
rather than allow it to remain until liveness detection removes it ([Section
6.2][s6.2]).  Releasing a registration that has already been replaced MUST
NOT remove its replacement.  Replacement and the ending it causes MUST be
one atomic step ([STM-18][stm-18]); without atomicity, two connections
admitting at once may both be kept.  A broker listening on more than one
mapping applies this across both.

The two connections are distinguished by which was admitted later and not by
the sequences they presented.  [POL-6][pol-6] has already compared those: a
connection is admitted only after being answered Current, so the later
admission is necessarily the higher sequence, and reading it again would
decide on an agent-supplied number what arrival order settles on its own.
Rationale appears in [Appendix C.12][sC.12].

<a id="stm-12"></a>**STM-12 (Replacement visibility).**  A broker MUST make
every replacement performed under [STM-11][stm-11] available to its
operator, together with the agent identifier concerned.  The means is not
specified.  This information MUST NOT be transmitted to an agent.

<a id="stm-13"></a>**STM-13 (Announcement visibility).**  A broker MUST make
the announced set most recently received on a registered connection
available to its operator, together with the agent identifier and the host
designator.  The means is not specified.  A broker MUST NOT derive any
behaviour beyond that report from an announcement ([Section 3.1][s3.1]).

<a id="stm-14"></a>**STM-14 (Reconnection).**  An agent maintains a
reconnection bound B per broker.  B is initially 5 seconds.  A connection
that remained in SERVING for at least the agent's K ([Section 6.2][s6.2]) is
termed stable.  On the end of a stable connection, and on completion of the
joining procedure, B returns to 5 seconds.  The end of an overtaken
connection ([Section 6.3][s6.3]) leaves B unchanged.  On a failed race
([Section 6.3][s6.3]), and on the end of a connection that is neither
stable, nor overtaken, nor released on completing the joining procedure, B
doubles, to at most 60 seconds.  The delay following a stable connection is
drawn against B as returned.  The delay following a failed race or any other
connection is drawn against B as it stood before the doubling, and the
doubling reaches only later races.

Following a failed race or a connection that ended, an agent MUST NOT start
a further race to the same broker until a delay drawn uniformly at random
from the interval [B, 2B) has elapsed.  A connection released on completing
the joining procedure, an overtaken connection, and a connection released on
its operator's instruction, are exempt.  A connection that ended following
`ACCESS`(Denied) is not exempt, whichever party ended it.  Rationale appears
in [Appendix C.18][sC.18].

The delay is randomised because B starts at the same value on every agent.
A fixed delay would make a fleet reconnecting in step the ordinary outcome
of a broker restart rather than an unlucky one, and the agents with most
reason to reconnect are those that would arrive together.

<a id="stm-15"></a>**STM-15 (Sequence durability).**  An agent MUST record a
sequence durably before transmitting it.  An agent MUST NOT transmit a
sequence it has transmitted before.

Recording first is what makes a crash harmless.  A sequence recorded and
never transmitted is spent: [POL-6][pol-6] requires each claim to exceed the
last accepted rather than to follow it, so gaps are ordinary and an agent
that died mid-connection resumes by moving further forward, not by repeating
itself.

<a id="stm-16"></a>**STM-16 (Freeze visibility).**  A broker MUST make every
freeze it performs ([Section 4.2][s4.2]) available to its operator, together
with the agent identifier and the sequence presented.  The means is not
specified.  This information MUST NOT be transmitted to an agent.

[STM-12][stm-12] does not cover a freeze: a freeze is [POL-5][pol-5] applied
to an identifier whose sequence went backwards, and takes no registration.
A freeze is invisible to every agent by [SEC-7][sec-7], so an operator
report is the whole of what [SEC-6][sec-6] detects.  Without this
requirement a stolen keypair produces a silent outage and no signal
anywhere.

<a id="stm-21"></a>**STM-21 (Freeze failure visibility).**  A broker MUST
make every failure of [POL-5][pol-5] invoked in answer to Stale available to
its operator, together with the agent identifier.  The means is not
specified.  This information MUST NOT be transmitted to an agent.

An identifier whose freeze failed remains bound and remains reachable: the
next claim exceeding the recorded sequence is answered Current and admitted.
Nothing distinguishes such an identifier on the wire, every denial being
uniform ([SEC-7][sec-7]), so the operator report is the whole of the signal.

<a id="stm-17"></a>**STM-17 (No inbound surface).**  An agent MUST NOT
accept an inbound connection in the course of this protocol.  Every
connection an agent makes under [Section 4.3][s4.3] is one it establishes
itself, and the connection to its broker is likewise its own.

<a id="stm-18"></a>**STM-18 (Broker locality).**  A broker is one process
holding one record store.  The atomicity required by [POL-2][pol-2],
[POL-6][pol-6] and [STM-11][stm-11] is atomicity within that process.  A
deployment running more than one broker process against one population of
agents is outside the scope of this document; [Section 12.5][s12.5] states
what such a deployment forfeits.

## 10. Versioning

### 10.1. Version Rules

A protocol version is identified by a single integer ([Section 2.1][s2.1]).
A version fixes the whole of the protocol: encodings, frames, messages,
procedures, verdicts, selector types and host designators.  The parameters
of [Section 6.2][s6.2] are not among them, each role holding its own.  There
is no negotiation of features within a version.

The rules an agent applies before transmitting the version octet are not
among them either.  [TRN-2][trn-2] completes before any octet of the
protocol is transmitted, so a party the agent refuses there has declared no
version and is answered by no verdict.  A change to [TRN-2][trn-2] therefore
does not change a protocol version, and cannot be announced through one
([Appendix C.21][sC.21]).

<a id="ver-1"></a>**VER-1 (Declaration).**  Every connection runs exactly
one protocol version, selected by the agent and declared as the first octet
it transmits on the control stream ([TRN-11][trn-11]).

<a id="ver-2"></a>**VER-2 (Supported range).**  A broker implements a
contiguous range of versions and answers each connection under the version
its first octet declares.  A broker MUST answer a version outside that range
with `ACCESS`(Denied), encoded as this document encodes it ([Section
10.2][s10.2]), and MUST then end the connection.  The octet 0x00 is not a
version ([Section 2.1][s2.1]) and is outside every range.  A broker MUST NOT
reinterpret the connection under another version and MUST NOT renegotiate a
version within a connection.

<a id="ver-3"></a>**VER-3 (Authenticated retry).**  An agent MAY retry using
a lower version it implements, over a new connection, only following
`ACCESS`(Denied) received from a broker whose identity has been verified per
[TRN-2][trn-2].  Within one invocation of a procedure, an agent MUST NOT
attempt any version more than once.  An agent MUST NOT change version in
response to a transport failure.  Each retry connection is subject to
[STM-14][stm-14].

<a id="ver-4"></a>**VER-4 (Operator visibility).**  A broker MUST make the
version declared by every connection it denies under [VER-2][ver-2]
available to its operator, distinguished from a malformed frame.  The means
is not specified.  This information MUST NOT be transmitted to the agent.

<a id="ver-5"></a>**VER-5 (Mapping scope).**  A transport mapping is
specific to a protocol version.  Appendices [A][appendix-a] and
[B][appendix-b] specify the mappings for PTP/1.

<a id="ver-6"></a>**VER-6 (Deployment minimum).**  The lowest version a
broker implements is the deployment's minimum.  No agent, transport or third
party can cause a broker to accept a version below it.  A version found to
be defective is withdrawn by raising that minimum.  Corrections to a
published version MUST be issued as a new version.

### 10.2. Version-Invariant Rules

<a id="ver-7"></a>**VER-7 (Version invariants).**  Every future version MUST
preserve each rule below.  An agent and a broker implementing disjoint
versions thereby fail deterministically.

| Rule | Preserved |
|------|-----------|
| Version first | The first octet an agent transmits on the control stream is the protocol version. |
| Broker-first proof | An agent transmits no protocol message before the broker's identity is proven ([TRN-2][trn-2]). |
| Uniform denial | A version outside the broker's range is answered with the `ACCESS` frame of this document, verdict Denied ([Section 2.4][s2.4]), and the connection then ends. |
| Authenticated retry | The retry discipline of [VER-3][ver-3] holds. |

### 10.3. Reserved Extensions

The following are reserved for future versions and MUST NOT be used by a
PTP/1 implementation.

**Selector types.**  Values other than the six of [Section 2.3][s2.3].  A
version defining a further selector type allocates it from the same
sequence, and states whether it names one service or names no service
([Section 2.3][s2.3]).  That statement fixes where the type may appear:
[Section 2.5][s2.5] admits a selector to a tunnel-stream preamble on that
property, and not on a list of type values.

No range over `unix-path` is anticipated.  A range is bounded by an ordering
over the values it spans, and a port range by the ordering of a 16-bit
integer; filesystem paths carry no ordering under which a low path and a
high path name a set an agent or an operator can predict.  A set of paths an
agent enumerates is announced as the paths themselves, and one it does not
enumerate is announced as `unix-path-any` ([Section 2.3][s2.3]).

**Host designators.**  Values other than 0x01, in `ANNOUNCE` and in the
tunnel-stream preamble alike ([Section 2.5][s2.5]).  A designator is a type
octet, not a host: as with a selector type ([Section 2.3][s2.3]), its value
fixes the form of the host value that follows it, and 0x01, the one
designator PTP/1 defines, is followed by no host value.  A version naming
concrete hosts defines further designators whose values carry an address or
a name, and revises the encodings that carry them; a version fixes the whole
of the protocol ([Section 10.1][s10.1]), so the reservation binds the
meaning of the octet and its position, not the layout around it.  The two
places are reserved together: such a version announces the hosts an agent
fronts and names one where each stream is asked for.  It thereby lets an
agent front a host that is not its own, which [SEC-5][sec-5] presently
forbids and which that version would have to say how to authorise.

**Frame types.**  Values other than the six of [Section 2.4][s2.4].

**Native datagram realisation.**  Under PTP/1, `udp-port` datagrams cross a
stream as records ([Section 2.5][s2.5]) on both mappings.  A future version
MAY realise `udp-port` over QUIC's unreliable datagram extension
[RFC9221][rfc9221]; such a version breaks the equivalence of the two
mappings and owes the downgrade analysis of [Section 12.7][s12.7] anew.

## 11. Conformance

An implementation claims conformance by satisfying the following behavioural
tests.  The tests are executable against an implementation in another
language, which is what makes conformance a property two implementations can
be compared on.  Every test in this section executes once per transport
mapping the implementation claims; an implementation claiming both mappings
passes the list over each.

<a id="cnf-1"></a>**CNF-1 (Stream reaches service).**  A joined agent
serving more than one selector: data transmitted over a stream opened for
each of them reaches the corresponding service and returns.

<a id="cnf-2"></a>**CNF-2 (Unserved selector refused).**  A stream opened
for a selector for which the agent's [POL-8][pol-8] returns no target is
answered `STREAM_REFUSE`, the consumer interface reports a refusal ([Section
6.1][s6.1]), and no data reaches any service on the agent's host as a result
of asking.  The selector's presence in the announced set does not change the
answer ([Section 3.1][s3.1]).

<a id="cnf-3"></a>**CNF-3 (Join binding).**  A valid join token binds the
keypair proven on the same connection.  An incorrect token, a token issued
for another agent identifier, and an expired token each bind nothing and
modify no record.  Repeating the joining procedure succeeds ([Section
5.2][s5.2]).

<a id="cnf-4"></a>**CNF-4 (Denial without registration).**  A claim for an
unjoined identifier, and a claim presenting a keypair other than the bound
one, are each denied, and neither results in a registration.

<a id="cnf-5"></a>**CNF-5 (Unpinned broker aborted).**  A broker presenting
a chain whose broker root key fingerprint differs from the pin causes the
agent to abort the handshake before transmitting any protocol message.

<a id="cnf-6"></a>**CNF-6 (Registration replacement).**  A replacement
registration takes effect immediately, and release of the registration it
replaced does not remove it.

<a id="cnf-7"></a>**CNF-7 (Idle connection survives).**  A connection
carrying no stream data for longer than (M+1)*K remains registered, both
roles having answered probes.

<a id="cnf-8"></a>**CNF-8 (Silent peer deregistered).**  A peer that has
ceased answering is deregistered within the bound of [Section 6.2][s6.2]
while the broker continues to transmit to it.

<a id="cnf-9"></a>**CNF-9 (Unsupported version denied).**  A version octet
outside the broker's range is answered `ACCESS`(Denied) and the connection
then ends.  The version is available to the operator, distinguished from a
malformed frame, and is not transmitted to the agent ([VER-2][ver-2],
[VER-4][ver-4]).

<a id="cnf-10"></a>**CNF-10 (Malformed frame ends connection).**  A frame
with an unknown type, a `CLAIM` of the wrong length, a sequence of 0, a port
of 0, a duplicate selector, and a second `CLAIM` each end the connection,
advance no state and modify no record ([STM-3][stm-3], [STM-10][stm-10]).

<a id="cnf-11"></a>**CNF-11 (Withdrawal restores joining).**  Following
withdrawal, a claim presenting the withdrawn keypair is denied, and the
joining procedure with a token issued after withdrawal binds a new keypair.
A join token presented for an identifier with a keypair already bound binds
nothing and modifies no record ([POL-5][pol-5], [POL-2][pol-2]).

<a id="cnf-12"></a>**CNF-12 (Denial is uniform).**  An unjoined identifier,
an identifier unknown to the broker, and a keypair other than the bound one
produce answers indistinguishable in content, in transport realisation, and
in timing ([SEC-7][sec-7]).  The timing this test observes is the delay of
[TRN-10][trn-10]; the delay conceals the three cases only while each policy
answers within it, so the test holds only under that condition ([Section
12.8][s12.8]).

<a id="cnf-13"></a>**CNF-13 (Stale sequence freezes).**  A claim whose
sequence does not exceed the last accepted is denied, and thereafter every
claim for that identifier is denied, including one from the keypair and
sequence that were current a moment before.  A claim skipping sequences is
admitted.

<a id="cnf-14"></a>**CNF-14 (Sequence needs the key first).**  A claim
presenting a low sequence with a keypair other than the bound one is denied
and freezes nothing: the identifier is still reachable afterwards by its own
agent ([POL-6][pol-6], [Section 4.2][s4.2]).

<a id="cnf-15"></a>**CNF-15 (Admission displaces connection).**  An
identifier that admits a second connection has the first ended, and a third
has the second ended, so no more than one connection is ever held for it
following admission.  A connection admitted for another identifier is
unaffected ([STM-11][stm-11]).

<a id="cnf-16"></a>**CNF-16 (Concurrent streams).**  A stream held open and
relaying does not prevent a second stream from being opened and relaying on
the same connection ([TRN-6][trn-6]).  Every test above opens its streams
one at a time, and an implementation that served streams one after another
is distinguishable from a conforming one only where two are open at once.

<a id="cnf-17"></a>**CNF-17 (Datagram boundaries preserved).**  A stream
opened for a `udp-port` selector relays datagrams in both directions with
boundaries preserved, including an empty datagram, and a datagram is never
split or merged ([Section 2.5][s2.5]).

<a id="cnf-18"></a>**CNF-18 (Socket path reaches service).**  A stream
opened for a `unix-path` selector reaches the UNIX-domain socket the agent's
[POL-8][pol-8] returns for it, and data returns over it.

<a id="cnf-19"></a>**CNF-19 (Denied agent delays reconnection).**  An agent
whose claim is denied during serving establishes no further connection to
that broker within 5 seconds of the connection's end, and a further denial
lengthens the delay ([STM-14][stm-14]).

<a id="cnf-20"></a>**CNF-20 (Announcement decides nothing).**  An `ANNOUNCE`
is answered by no message, and a connection that sends one goes on serving.
A second one, naming a different set, changes what a stream is answered by
in no way: a selector absent from the announced set is served where
[POL-8][pol-8] has a target for it, and one present in it is refused where
[POL-8][pol-8] has none ([Section 3.1][s3.1], [STM-13][stm-13]).

<a id="cnf-21"></a>**CNF-21 (Forwards past the ceiling).**  Where a mapping
bounds the tunnel streams one connection carries, an open past that bound is
answered as a refusal within the bound of [TRN-6][trn-6], the connection
goes on carrying the streams it already has, and closing them lets a further
open succeed.  A consumer asking for more than a connection can hold does
not cost an agent its tunnel.

<a id="cnf-22"></a>**CNF-22 (Joining recovers a frozen identifier).**  An
identifier frozen by a sequence that went backwards admits a claim carrying a
sequence below the one that froze it, once a keypair has been bound to it
again under [POL-2][pol-2].  A freeze at the largest sequence representable
is recovered no differently from any other ([POL-6][pol-6]).

<a id="cnf-23"></a>**CNF-23 (An announced set is not dialable).**  An agent
announcing a `tcp-port-range`, a `udp-port-range` or a `unix-path-any`
serves a stream opened for a selector that one identifies and for which
[POL-8][pol-8] returns a target.  A stream whose preamble carries one of
those three selectors is closed without an answer octet, which is neither
`STREAM_ACCEPT` nor `STREAM_REFUSE`; the connection goes on carrying the
streams it already has ([Section 2.5][s2.5]).

<a id="cnf-24"></a>**CNF-24 (Uncertified serving key aborted).**  A broker
presenting the pinned broker root key together with a certificate that key
did not sign causes the agent to abort the handshake before transmitting any
protocol message.

<a id="cnf-25"></a>**CNF-25 (Superseded serving key aborted).**  An agent
that has accepted a broker serving key certified at one rollback index, and
has then accepted one certified at a higher index, aborts the handshake
before transmitting any protocol message on being presented the first again,
the broker root key and its signature being unchanged.

<a id="cnf-26"></a>**CNF-26 (Unchanged index admitted).**  An agent
reconnecting to a broker that has not been restarted is presented the
rollback index it has already accepted, and completes the handshake.

<a id="cnf-27"></a>**CNF-27 (Nothing accepted admits any index).**  An agent
that has accepted no rollback index completes the handshake against a broker
certifying any rollback index within the range of [TRN-13][trn-13].

<a id="cnf-28"></a>**CNF-28 (Oversized index aborted).**  A broker
presenting a certificate the pinned broker root key did sign, whose
serialNumber is wider than eight octets, causes the agent to abort the
handshake before transmitting any protocol message.

## 12. Security Considerations

### 12.1. Properties

Given a conforming transport implementation and a conforming policy, PTP/1
provides:

<a id="sec-1"></a>**SEC-1 (Mutual authentication).**  An agent communicates
only with a broker proving possession of a serving key that the pinned broker
root key certified ([TRN-2][trn-2]).  A broker admits an agent identifier
only on proof of possession of the keypair bound to it ([TRN-3][trn-3],
[POL-1][pol-1]), or,
where none is bound, of a keypair that an unexpired join token issued for
that identifier authorises it to bind ([POL-2][pol-2], [SEC-3][sec-3]).

<a id="sec-2"></a>**SEC-2 (No listening surface).**  An agent binds no
listening port in the course of this protocol ([STM-17][stm-17]).  The sole
inbound path it defines is a stream over the connection the agent itself
established.

<a id="sec-3"></a>**SEC-3 (Join integrity).**  A bound keypair is one whose
possession was proven on the connection that carried a valid join token, and
the binding is committed before `ACCESS`(Granted) is transmitted.

<a id="sec-4"></a>**SEC-4 (Single-use bootstrap).**  A join token observed
after use confers nothing.  An unused token expires.

<a id="sec-5"></a>**SEC-5 (Least reach).**  A stream terminates at the
target returned by [POL-8][pol-8] for its selector, which is a service on
the agent's own host, or at nothing.

<a id="sec-6"></a>**SEC-6 (Detectable compromise).**  A keypair held by two
parties is detected no later than the next claim from whichever holder has
fallen behind.  The two need not be connected at once.  Every claim carries
a sequence that must exceed the last accepted ([Section 2.1][s2.1],
[POL-6][pol-6]), and two parties cannot both keep pace with one sequence:
whichever falls behind presents a number already seen, and the identifier is
frozen for both ([Section 1.2][s1.2]).

That bound is the whole of the property, and three cases fall outside it.  A
holder presenting a sequence far ahead of the other is admitted until the
other claims, so this neither prevents use of a copied keypair nor shortens
the period of use.  A holder that never claims again is never detected: a
keypair copied from a host that is subsequently decommissioned is
indistinguishable from that host, and nothing available to a broker
distinguishes them.  Where a broker discards its records on restart
([POL-6][pol-6]), two claims separated by one are not compared unless the
earlier holder has claimed again since, so a holder that connects rarely
enough to fall either side of a restart is detected only once it and the
other are both claiming within one run.

Concurrent use of one keypair remains visible as repeated replacement
([STM-11][stm-11], [STM-12][stm-12]), which is not what detection rests on.
Rationale appears in [Appendix C.14][sC.14].

<a id="sec-7"></a>**SEC-7 (No record disclosure).**  A party that has not
proven possession of a bound keypair learns nothing of the broker's records.
`ACCESS`(Denied) does not distinguish an unknown agent identifier from a
known one, nor either from a keypair that is not the bound one ([Section
3.2][s3.2], [TRN-10][trn-10]).  In content this holds of every deployment;
in timing it rests on the delay of [TRN-10][trn-10] covering what
[POL-1][pol-1] costs, which is a property of a deployment rather than of
this document ([Section 12.8][s12.8]).

The announced set is not among those records: an agent announces it to the
broker ([Section 3.1][s3.1]), so a broker knows it without asking.  Neither
is the version range: a connection denied under [VER-2][ver-2] then ends, so
a party probing versions learns which the broker implements, and nothing
else.  `STREAM_REFUSE` does not distinguish a selector [POL-8][pol-8] has no
target for from a service that cannot be reached ([Section 4.3][s4.3]); that
distinction is all a broker has left to learn.

### 12.2. Threat Analysis

| Capability held by attacker | Obtained | Bounded by |
|---|---|---|
| Passive network position | Traffic analysis | [TRN-4][trn-4] |
| Active network position | Suppression of the traffic crossing its position, for as long as it holds that position | No persistent effect: service resumes when the position is lost.  It cannot forge `ACCESS`, cannot freeze an identifier ([POL-6][pol-6] is reached only through proof of possession, [TRN-3][trn-3]), cannot induce version retry ([VER-3][ver-3]), and obtains no more than the other mapping's transport properties by blocking one ([Section 12.7][s12.7]) |
| An agent identifier | Nothing; identifiers are public, and every claim is answered alike | [SEC-1][sec-1], [SEC-7][sec-7] |
| An unused join token | Ability to join a host of the attacker's choosing under that identifier, where no keypair is yet bound | Expiry; single use; withdrawal ([POL-5][pol-5]) |
| A used join token | Nothing | [SEC-4][sec-4] |
| The broker's record store | Commitments and public keys; no tokens and no private keys | [POL-2][pol-2], [POL-9][pol-9] |
| An agent private key | Impersonation of that agent, on either mapping and any version the broker implements | [SEC-6][sec-6]; [VER-6][ver-6]; withdrawal ([POL-5][pol-5]) and re-joining the affected agent |
| An agent private key, used to freeze | Denial of the tunnel to the agent, by claiming a sequence it cannot then exceed ([POL-6][pol-6]).  The cost is the binding, and recovery is an operator issuing a further token | Not bounded by this protocol.  It is the price of [SEC-6][sec-6]: a broker that cannot tell two holders apart cannot deny one without denying both ([Appendix C.11][sC.11]) |
| A joined but hostile agent | Streams opened to it terminate wherever it chooses; nothing beyond its own record | It cannot open streams and cannot reach other agents ([Section 4.3][s4.3]).  [STM-11][stm-11] bounds it to one admitted connection.  [SEC-5][sec-5] binds conforming agents only |
| A hostile or compromised consumer | Streams to every registered agent, at every selector that agent's [POL-8][pol-8] serves | Not bounded by this protocol; a consumer is within the broker's trust boundary ([Section 12.3][s12.3]) |
| A defective version within the broker's range | The defect, on connections selecting that version | [VER-6][ver-6] |
| A broker serving key private key | Impersonation of the broker to every agent that has not yet met a higher rollback index, on either mapping | Replacement of the broker serving key at a higher rollback index, certified under the broker root key and reaching every agent in the handshake ([TRN-2][trn-2], [TRN-13][trn-13], [Section 12.4][s12.4]).  The exposure ends for each agent at that agent's first connection to the broker after the replacement, and not at the replacement itself |
| The broker root key private key | Impersonation of the broker to every agent, on either mapping | Not bounded by this protocol; recovery is re-provisioning every agent ([Section 12.4][s12.4]) |
| The agent host | Everything that host serves | Out of scope |
| The broker host | Every connection and every policy answer | Out of scope |
| Traffic volume | Connection exhaustion at the broker | [TRN-10][trn-10] bounds unadmitted connections and their holders; volumetric denial of service against the network is out of scope |

### 12.3. The Consumer

This document authenticates the agent to the broker and the broker to the
agent.  It does not authenticate a consumer to either.  A party able to
invoke the consumer interface ([Section 1.3][s1.3]) reaches every registered
agent at every selector for which that agent's [POL-8][pol-8] returns a
target, and the broker relays each such stream in plaintext at its own
position: the broker terminates one conversation and originates another
([Appendix C.9][sC.9]), so stream contents are visible to it by
construction.

The protections of this document are against a party impersonating an agent,
and against a party reaching an agent's tunnel without possession of the
bound keypair.  Authorisation of consumers, and protection of the path
between a consumer and the broker, are the broker environment's to supply.
A deployment in which consumers are not all trusted equally requires an
authorisation layer this document does not define.  A future version
defining host designators beyond the agent's own host ([Section
10.3][s10.3]) enlarges what a consumer reaches, and owes this boundary a
re-examination.  Rationale appears in [Appendix C.16][sC.16].

### 12.4. Fixed Identities

The broker root key, and each agent identity, is fixed: this document defines
no rotation mechanism for either.  A broker root key is replaced only by
re-provisioning every agent's pin out of band; an agent keypair is replaced
only by withdrawal and a further joining procedure ([POL-5][pol-5], [Section
5.2][s5.2]).

A broker serving key is not fixed.  A broker replaces one at any time, by
certifying the replacement under the broker root key at a rollback index
higher than any it has certified before ([TRN-13][trn-13]), and an agent that
was offline throughout the replacement receives the new certificate in the
handshake on its next connection ([TRN-2][trn-2]).  No agent is contacted,
and no window exists during which an agent must be connected.

Compromise of a broker serving key private key is therefore recovered within
the protocol.  The rollback index is what recovers it: nothing in this
protocol revokes a certificate, and an agent evaluates no validity period, so
the certificate the compromised key is carried in stays signed by the broker
root key for ever, and an agent distinguishes it from the replacement by its
rollback index and by nothing else.

The exposure ends for one agent when that agent first connects to the broker
after the replacement, and it ends for the deployment when the last agent
has.  Replacement alone ends it for no agent: an agent holding a connection
reads no certificate until that connection ends ([Section 4.1][s4.1]), and
this document defines no idle timeout on either role ([Section 6.2][s6.2],
[TRN-8][trn-8]).  A broker restart is therefore the coarsest unit in which
the exposure moves at all, ending every connection and so obliging every
agent to handshake again.  A deployment bounds the exposure in time by the
interval between restarts, and by nothing shorter.

An agent that never connects to the broker after a replacement holds the
superseded rollback index indefinitely, and accepts the compromised
certificate from any party that can both present it and occupy the broker's
address.  This residue is not bounded by this protocol.  Bounding it would
require the agent to learn of the replacement without connecting, which is
[Appendix C.15][sC.15]'s case, or to evaluate a validity period, which
[Appendix C.21][sC.21] rejects.

Compromise of the broker root key private key has no in-protocol recovery.
This is a stated decision rather than an omission: a compromised trust anchor
forces human intervention in every design.  Rationale appears in [Appendix
C.15][sC.15].

### 12.5. Broker Locality

[STM-18][stm-18] confines a broker to one process over one record store.
The guarantees of [POL-6][pol-6] and [STM-11][stm-11] are stated relative to
that process: the restart argument of [POL-6][pol-6] assumes a restart ends
every connection, and the atomic compare-and-record assumes one store.

A deployment that replicates a broker without making those operations atomic
across replicas forfeits [SEC-6][sec-6] silently: two holders of one keypair
alternating between replicas are compared against divergent records and
neither is ever seen to go backwards.  Freezing still propagates wherever
the binding store is shared, but the trigger never fires.  Replication is
out of scope; a deployment that requires it owes each atomicity of this
document a distributed equivalent.  Rationale appears in [Appendix
C.17][sC.17].

### 12.6. Version Downgrade

An attacker in the network cannot induce version retry: retry follows only
`ACCESS`(Denied) received over a connection satisfying [TRN-2][trn-2], and
never a transport failure ([VER-3][ver-3], [STM-10][stm-10]).  Interference
is therefore limited to denial of service.

An agent may select any version within a broker's range, including the
lowest.  The bound on this is [VER-6][ver-6]; a defective version remains
reachable for as long as an operator retains it.  A denial gives the agent
no reason: an agent that retries lower versions after one does so once per
version it implements ([VER-3][ver-3]), and the walk is bounded by the
agent's own version count.

### 12.7. Mapping Downgrade

A party that blocks UDP forces an agent configured per [Section 6.3][s6.3]
onto the mapping of [Appendix B][appendix-b].  The two mappings realise the
identical protocol, including the `udp-port` relay ([Section 2.5][s2.5]), so
the forcing party obtains no change in any property of [Section
12.1][s12.1].  What changes is transport behaviour: over TCP a single lost
segment stalls every stream on the connection until it is retransmitted,
where QUIC recovers streams independently.  Degraded latency under loss is
the whole of the gain.

A future version realising `udp-port` natively ([Section 10.3][s10.3])
breaks this equivalence and owes this analysis anew.

### 12.8. Timing of Denials

[SEC-7][sec-7] holds in content unconditionally and in timing conditionally.
The cases it covers are answered by the same frame, over the same transport,
at a moment [TRN-10][trn-10] fixes on receipt of the attempt; what varies
between them is the work [POL-1][pol-1] does, and the fixed moment conceals
that work only while the work finishes before it.

Nothing here bounds a policy.  Where records are consulted over a network,
or under a load a deployment did not plan for, an attempt can be answered
after the delay has passed, and the answer then carries whatever the policy
cost.  The cases are separable to the extent those costs differ.  An
operator to whom the distinction matters keeps the policy's slowest case
inside the delay; an implementation that instead extended the delay would
disclose the same difference, one delay later ([TRN-10][trn-10]).

The distinction disclosed is the one [SEC-7][sec-7] names and no more:
whether an identifier is known, and whether a keypair is the bound one.  No
record content follows from it, and possession of a bound keypair remains
the only thing that admits a party.

### 12.9. Unenumerated Announcements

A `tcp-port-range`, a `udp-port-range` and a `unix-path-any` change what an
operator is told and nothing else ([Section 3.1][s3.1]).  What a consumer
reaches is what [POL-8][pol-8] returns a target for, and a policy answering
every selector of a type is a decision of the deployment rather than of this
document.

Such a policy is bounded by the agent's own credentials and by nothing in
this protocol.  For `tcp-port` and `udp-port` the bound is the services
listening on the agent's host.  For `unix-path` the bound is every socket
the agent's process can open, across a filesystem namespace whose members
the agent does not enumerate and whose privileges differ from one another: a
socket that authorises its peer by credential grants whatever its service
grants to the agent.  A deployment whose [POL-8][pol-8] returns a target for
every `unix-path` therefore gives its consumers the agent's own authority
over every such service, and this document places no requirement between a
broker and its consumers ([Section 12.3][s12.3]).

[SEC-5][sec-5] is unaffected.  A target remains a service on the agent's own
host ([POL-8][pol-8]), and no selector type of this document names another
host ([Section 10.3][s10.3]).

## 13. IANA Considerations

This document has no IANA actions.

The ALPN protocol identifier "ptp1" ([TRN-12][trn-12]) is not registered in
the IANA "TLS Application-Layer Protocol Negotiation (ALPN) Protocol IDs"
registry; a deployment of this protocol interoperates only with its own
brokers and agents.  Selector types, host designators and frame types are
assigned by revisions of this document ([Section 10.3][s10.3]).

## 14. References

### 14.1. Normative References

- **[RFC2104][rfc2104]** — Krawczyk, H., Bellare, M., and R. Canetti, "HMAC:
  Keyed-Hashing for Message Authentication", RFC 2104, February 1997.
- **[RFC2119][rfc2119]** — Bradner, S., "Key words for use in RFCs to
  Indicate Requirement Levels", BCP 14, RFC 2119, March 1997.
- **[RFC5280][rfc5280]** — Cooper, D., Santesson, S., Farrell, S., Boeyen,
  S., Housley, R., and W. Polk, "Internet X.509 Public Key Infrastructure
  Certificate and Certificate Revocation List (CRL) Profile", RFC 5280, May
  2008.
- **[RFC6234][rfc6234]** — Eastlake 3rd, D. and T. Hansen, "US Secure Hash
  Algorithms (SHA and SHA-based HMAC and HKDF)", RFC 6234, May 2011.
- **[RFC7301][rfc7301]** — Friedl, S., Popov, A., Langley, A., and E.
  Stephan, "Transport Layer Security (TLS) Application-Layer Protocol
  Negotiation Extension", RFC 7301, July 2014.
- **[RFC8174][rfc8174]** — Leiba, B., "Ambiguity of Uppercase vs Lowercase
  in RFC 2119 Key Words", BCP 14, RFC 8174, May 2017.
- **[RFC8446][rfc8446]** — Rescorla, E., "The Transport Layer Security (TLS)
  Protocol Version 1.3", RFC 8446, August 2018.
- **[RFC9000][rfc9000]** — Iyengar, J., Ed. and M. Thomson, Ed., "QUIC: A
  UDP-Based Multiplexed and Secure Transport", RFC 9000, May 2021.
- **[RFC9001][rfc9001]** — Thomson, M., Ed. and S. Turner, Ed., "Using TLS
  to Secure QUIC", RFC 9001, May 2021.
- **[RFC9562][rfc9562]** — Davis, K., Peabody, B., and P. Leach,
  "Universally Unique IDentifiers (UUIDs)", RFC 9562, May 2024.
- **[YAMUX][yamux]** — Hashimoto, M., "Yamux Specification", HashiCorp,
  https://github.com/hashicorp/yamux/blob/master/spec.md.

### 14.2. Informative References

- **[IEEE802.1AR][ieee8021ar]** — IEEE, "IEEE Standard for Local and
  metropolitan area networks — Secure Device Identity", IEEE Std 802.1AR-2018,
  August 2018.
- **[RFC4787][rfc4787]** — Audet, F., Ed. and C. Jennings, "Network Address
  Translation (NAT) Behavioral Requirements for Unicast UDP", BCP 127, RFC
  4787, January 2007.
- **[RFC5382][rfc5382]** — Guha, S., Ed., Biswas, K., Ford, B., Sivakumar,
  S., and P. Srisuresh, "NAT Behavioral Requirements for TCP", BCP 142, RFC
  5382, October 2008.
- **[RFC7030][rfc7030]** — Pritikin, M., Ed., Yee, P., Ed., and D.  Harkins,
  Ed., "Enrollment over Secure Transport", RFC 7030, October 2013.
- **[RFC8305][rfc8305]** — Schinazi, D. and T. Pauly, "Happy Eyeballs
  Version 2: Better Connectivity Using Concurrency", RFC 8305, December
  2017.
- **[RFC8555][rfc8555]** — Barnes, R., Hoffman-Andrews, J., McCarney, D.,
  and J. Kasten, "Automatic Certificate Management Environment (ACME)", RFC
  8555, March 2019.
- **[RFC9113][rfc9113]** — Thomson, M., Ed. and C. Benfield, Ed., "HTTP/2",
  RFC 9113, June 2022.
- **[RFC9221][rfc9221]** — Pauly, T., Kinnear, E., and D. Schinazi, "An
  Unreliable Datagram Extension to QUIC", RFC 9221, March 2022.
- **[TUF][tuf]** — Cappos, J., Kuppusamy, T. K., Moore, J., Awwad, M., and
  L. DeLong, "The Update Framework Specification", version 1.0.38.
- **[UPTANE][uptane]** — Uptane Alliance, "Uptane Standard for Design and
  Implementation", version 2.1.0.

---

## Appendix A. QUIC Transport Mapping (Normative)

This appendix is normative for implementations using QUIC as the transport.
It realises PTP/1 ([VER-5][ver-5]).

**Establishment.**  A connection is QUIC version 1 [RFC9000][rfc9000],
secured with TLS 1.3 as [RFC9001][rfc9001] specifies.  The ALPN identifier
is "ptp1" ([TRN-12][trn-12]).  Certificates, pin verification and client
certificate handling follow [TRN-2][trn-2] and [TRN-3][trn-3].  Session
tickets, pre-shared keys and 0-RTT are excluded by [TRN-4][trn-4]: a broker
MUST NOT send NewSessionTicket, and early data is never enabled.

**Control stream.**  The first client-initiated bidirectional stream.  The
agent opens it on handshake completion and transmits the version octet
([TRN-11][trn-11]) followed by frames ([Section 2.4][s2.4]).

**Tunnel streams.**  Server-initiated bidirectional streams.  `STREAM_OPEN`
is realised by opening the stream and transmitting the selector; the answer
octet and the relay follow [Section 2.5][s2.5].

**Default deny ([TRN-9][trn-9]).**  A broker permits the agent one
bidirectional stream and no unidirectional streams: it announces an initial
maximum of 1 bidirectional and 0 unidirectional streams and does not raise
the agent's bidirectional limit.  The DATAGRAM extension [RFC9221][rfc9221]
MUST NOT be negotiated ([Section 10.3][s10.3]).  An agent announces 0
unidirectional streams and bounds broker-opened bidirectional streams by
announcing a concurrency limit of its own choosing.

**Liveness ([TRN-8][trn-8]).**  Probes are the `PING` frames of [Section
6.2][s6.2], on the control stream.  QUIC's own keep-alive is not used.  A
QUIC idle timeout, where one is configured at all, MUST NOT be shorter than
the role's detection bound.

**Address validation ([TRN-10][trn-10]).**  A broker MUST use Retry while
the unadmitted-connection bound is approached; Retry is the validation
mechanism the transport supplies.

**Migration.**  An agent MAY use QUIC connection migration.  A migrated
connection is the same connection: no state of [Section 4][s4] changes, and
no replacement under [STM-11][stm-11] occurs.

**End of connection.**  A deliberate end is an application CONNECTION_CLOSE.
Flow control is QUIC's per-stream flow control ([TRN-6][trn-6]).

## Appendix B. TLS over TCP Transport Mapping (Normative)

This appendix is normative for implementations using TLS over TCP as the
transport.  It realises PTP/1 ([VER-5][ver-5]).

**Establishment.**  A connection is TLS 1.3 over TCP [RFC8446][rfc8446].
The ALPN identifier is "ptp1" ([TRN-12][trn-12]).  Certificates, pin
verification and client certificate handling follow [TRN-2][trn-2] and
[TRN-3][trn-3].  Session tickets, pre-shared keys and early data are
excluded by [TRN-4][trn-4].

**Multiplexing.**  A yamux session [YAMUX][yamux] runs over the TLS
connection.  The agent is the yamux client and opens odd-numbered streams;
the broker is the yamux server and opens even-numbered streams.

**Control stream.**  The first agent-opened yamux stream.  The agent opens
it on session establishment and transmits the version octet
([TRN-11][trn-11]) followed by frames ([Section 2.4][s2.4]).

**Tunnel streams.**  Broker-opened yamux streams, per [Section 2.5][s2.5].

**Default deny ([TRN-9][trn-9]).**  An agent MUST NOT open a yamux stream
beyond the control stream; a broker receiving one ends the connection
([STM-3][stm-3]).

**Liveness ([TRN-8][trn-8]).**  Probes are the `PING` frames of [Section
6.2][s6.2], on the control stream.  A role MUST answer a yamux ping frame it
receives and MUST NOT rely on originating one; no inactivity timeout is
configured at the yamux or TCP layer.

**Flow control ([TRN-6][trn-6]).**  yamux per-stream windows, which MUST
remain enabled.

**Address validation ([TRN-10][trn-10]).**  The transport supplies none, so
the occupancy mechanisms of [TRN-10][trn-10] carry the whole of the
per-source requirement.  TCP-level protections of the host environment are
outside this document.

**End of connection.**  A deliberate end is a yamux GoAway followed by TLS
close_notify.  Over TCP, a lost segment stalls every stream until it is
retransmitted ([Section 12.7][s12.7]).

---

## Appendix C. Design Rationale (Informative)

This appendix records the reasoning behind decisions in the normative text.
It is not required for interoperability.

### C.1. Two Verdicts

`ACCESS` verdicts describe broker-side records rather than instructing the
transport.  A third verdict suggests itself: Pending, meaning that the
claimed identifier is known and has no keypair bound to it.  No agent would
learn anything from it: an agent invoking [Section 5.2][s5.2] was given a
token out of band and holds one already, while an agent invoking [Section
5.1][s5.1] would treat Pending and Denied alike as failure.  Pending would
change no agent's behaviour and would disclose the contents of the broker's
records to any party that asked ([SEC-7][sec-7]).  A joining agent therefore
transmits its token after a denial rather than after an invitation.  The
external account binding of ACME [RFC8555][rfc8555] is presented the same
way, unprompted, and no ACME server announces that an identity is known but
unenrolled.  EST [RFC7030][rfc7030] does challenge for its enrolment
credential, but challenges every request alike and discloses nothing about
which identities are known; PTP reaches the same end by denying uniformly.

### C.2. Idempotent Joining

The failure this addresses is a broker that commits a binding and answers
`ACCESS`(Granted), followed by an agent that terminates before recording its
own success.  The token is then spent.  Any stricter treatment leaves a
functioning host unable to proceed until an operator issues a further token.

### C.3. Replacement Rather Than Rejection

Admission is proof of possession of a keypair.  Two parties holding one
private key are indistinguishable to the broker, so no policy can prefer the
earlier connection on identity grounds.  Replacement additionally serves the
common case, which is an agent reconnecting after a change of network path
while its previous connection has not yet been detected as dead; preferring
the earlier connection would render such an agent unreachable until that
detection completed ([Section 6.2][s6.2]).

### C.4. Liveness Parameters Are Local

Fixed values of K and M, one pair for every implementation, are what safety
appears to require: with no parameter exchange, the two roles seem safe
together only where this document chooses for them.  That reading takes the
relation to be pairwise.  It is not.  What a role concludes is fed by what
that role receives, and what it receives includes the answers to its own
probes, so a role that probes every K and is answered receives every K
whatever its peer has chosen.  The reckoning closes over one role's own
values; nothing crosses the wire and nothing needs to agree.  An idle
timeout beside the probe count is dominated in every case, a peer that stops
answering being caught by the probe count first and a peer that answers
probes resetting any idle measure by doing so, and this document defines
none.

What makes this true is that both roles probe and both answer ([Section
6.2][s6.2]).  Locality is what a deployment needs anyway: an agent behind a
network address translator that expires mappings quickly shortens its own K,
and an agent on a metered link lengthens it.  The floor on K is the one
thing a role cannot be trusted to choose alone: a probe interval is a cost
imposed on the peer, and a fleet is a large multiplier on a small number.

### C.5. Absence of SHOULD

A requirement expressed as SHOULD is decided separately by each implementer.
Two implementations deciding it differently interoperate until the case in
which they differ arises.  Every requirement in this document is therefore
mandatory or an explicit permission.

### C.6. One Wire Protocol, Two Mappings

The alternative to this document's shape is a transport-agnostic protocol
behind a pluggable carrier abstraction, with existing protocols, SSH above
all, as bindings.  The abstraction costs twice.  A binding assembled from an
existing protocol's native operations cannot always carry this document's
semantics: an SSH global request names one port, so the atomic set of
`ANNOUNCE` would expand into one exchange per selector, with no marker for
where the set ends.  And the generality buys nothing: an agent pins its
broker's key end to end, so no terminating intermediary can ever sit on the
path, and the interoperability a carrier-agnostic design purchases is
exactly the interoperability pinning forbids.

This document instead defines one framing ([Section 2.4][s2.4]) and maps it
onto two transports that are deliberately equivalent.  `ANNOUNCE` is one
frame; the atomicity [Section 2.4][s2.4] states is carried by the encoding
rather than reconstructed by a mapping.  Equivalence is also what makes the
fallback of [Section 6.3][s6.3] sound.  An attacker who can induce a
transport failure chooses which mapping the agent lands on, so fallback
between unequal transports would be a downgrade the attacker selects.
Between equal mappings there is nothing to choose ([Section 12.7][s12.7]).

### C.7. The Choice of yamux

The mapping of [Appendix B][appendix-b] needs a multiplexer in which both
sides open streams and every stream has flow control.  HTTP/2
[RFC9113][rfc9113] is the one living IETF multiplexer over TCP, and its
stream initiation is wrong-shaped: only the client opens request streams,
and the broker must open streams toward the agent.  Running the roles
inverted is possible, and is how deployed reverse tunnels over HTTP/2 work,
but the cost lands in this document: a mapping would owe the inversion, the
request forms, and a default-deny clause for every HTTP construct left over,
while the benefits HTTP/2 is chosen for elsewhere, intermediaries above all,
are unavailable here because pinning forbids them ([Appendix C.6][sC.6]).
yamux supplies bidirectional opens, per-stream windows and a ping in a
specification of a page, exercised in its ordinary configuration.

### C.8. Datagrams over Streams

A `udp-port` stream carries records rather than native datagrams, on both
mappings, because the requirement is reachability: a UDP service behind the
agent is reachable at all, over whichever mapping a network permits.  Native
datagrams exist on one transport only ([RFC9221][rfc9221]), so realising
them natively would split the mappings into unequal ones and reopen the
downgrade question that equivalence closes ([Section 12.7][s12.7]).  What
framing over a reliable stream costs a UDP service is ordering and delay it
did not ask for: datagrams arrive late rather than dropped, and a loss
stalls those behind it.  A version that needs fidelity as well as
reachability defines the native realisation and pays for the split ([Section
10.3][s10.3]).

### C.9. A Stream Proxy, Not a Packet Tunnel

A stream terminates at the target [POL-8][pol-8] returns, and the agent
reaches that target by opening a connection of its own.  What crosses a
tunnel is the bytes of that conversation and never the packets carrying
them.  Carrying a reliable transport inside another one leaves two
retransmission timers unable to see each other, and throughput collapses
rather than degrading; a proxy that terminates one conversation and
originates another has one such timer per hop and cannot reach that state.
The cost is paid elsewhere, in sharing one connection's loss recovery across
streams ([Section 12.7][s12.7]), which the mapping of [Appendix
A][appendix-a] does not pay, its streams being separately recoverable.

### C.10. Withdrawal as a Separate Operation

[POL-5][pol-5] is the operation a wire-only reading of this protocol omits:
no message invokes it.  Without it, an agent whose key was lost is
unrecoverable, because [Section 5.2][s5.2] denies a keypair other than the
bound one whatever token accompanies it, and the threat analysis's bound on
a stolen private key, re-joining, would name an operation that does not
exist.  It is separate from issuance because the two carry different risk:
issuing a credential is routine, and destroying a working one takes a
functioning agent off the network.  Recovery therefore costs an operator two
actions, which is the intended price of a binding that no single mistake
dissolves.

### C.11. Freezing Rather Than Refusing

The obvious answer to a Stale sequence is to refuse the claim that went
backwards and let the other party carry on.  It inverts the outcome.  A
thief that copied an agent's disk holds its keypair and its sequence.  It
connects, presents the next number, and is admitted; the broker records it.
The real agent connects later, presents the same number, and is the one that
went backwards.  Refusing the stale claimant would lock out the agent and
leave the thief serving.  A broker cannot tell one from the other, since
both prove the same key ([Appendix C.3][sC.3]).  Freezing both is the only
answer available to a party that cannot distinguish them.

One consequence follows.  An agent restored from a snapshot carries a stale
sequence and is frozen, which is correct, since a restored agent and a
cloned one are indistinguishable from the broker's position.  It does make
rolling a machine back an operator's business rather than a silent recovery.

### C.12. Bounding Admitted Connections

[TRN-10][trn-10] bounds attempts, resource consumption and unadmitted
connections.  A party holding a bound keypair is outside all three: its
connections are admitted, it chooses its own sequences and is answered
Current on each ([POL-6][pol-6]), and it answers probes and so is never
reaped.  [STM-11][stm-11] is what bounds it: every admission takes the
registration for the identifier and ends the connection that held it before,
so such a party holds one admitted connection however many it opens.  The
bound is one rather than a value a deployment picks because an agent
identity is one machine.

Replacement at admission has one cost a conforming agent pays.  A joining
procedure invoked while the keypair is already bound is admitted on its
`CLAIM` ([STM-7][stm-7]) and displaces the connection that is serving; the
serving procedure observes its connection end and reconnects
([STM-14][stm-14]).  [STM-12][stm-12] makes the displacement visible, so a
recurring one is not mistaken for an agent reconnecting after a change of
network path.

### C.13. Bounding Who Holds Admission

A bound on unadmitted connections with no qualifier on their holders is
filled by whoever opens connections fastest.  Three mechanisms answer it and
they are not equivalent.  A deadline bounds how long one connection holds
one slot, which lowers what a socket buys but leaves occupancy untouched.
Accounting per source bounds concentration directly, and is defeated only by
a party with addresses to spend.  Refusing with a rising probability
degrades rather than failing: an agent arriving while the bound is nearly
full retains a chance of admission, where a flat bound gives it none once
reached.  Address validation removes one adversary and not the attack: a
transport that will not commit connection state until a peer has
demonstrated receipt at the address it claimed denies the bound to a party
spoofing addresses it does not hold, which is what Section 8 of
[RFC9000][rfc9000] provides Retry for; a party using addresses it does hold
pays one further round trip and occupies the bound as before.
[TRN-10][trn-10] therefore requires validation in addition to the occupancy
mechanisms and not in place of them.  The TCP mapping has no equivalent, so
under [Appendix B][appendix-b] the occupancy mechanisms carry the whole
requirement.

### C.14. Detection Without Overlap

Overlap is not what detection rests on.  A thief who uses the key only while
the agent is offline produces no contention, no replacement and no record of
anything, so a mechanism relying on concurrent use would miss the ordinary
theft.  Detection rests on the sequence [POL-6][pol-6] compares and not on
concurrency, so the two holders need not be connected at once
([SEC-6][sec-6]).

### C.15. Fixed Identities

Announcing a successor key over existing connections was considered and
rejected.  An announcement is a message, a message needs a connection, and a
connection needs the agent to accept the broker's key first, because
[TRN-2][trn-2] completes before an agent transmits anything.  The agent that
most needs a successor key is therefore the one that cannot be told it.  Any
announcement scheme has a window, and the set of agents it loses is the set
that never connects during that window.

Separating the key that is pinned from the key that is served has no window,
because it makes no announcement.  A certificate chain travels in the
handshake, so an agent that has been offline for a year receives the current
serving key at the moment it connects, checks one signature against the key
it has held since joining, and proceeds.  It learns nothing, stores nothing
and asks nobody.

What this costs is one long-lived key that signs and does not serve.  The
broker root key is loaded only to certify a serving key, which a broker does
once per start, so a deployment MAY hold it outside the broker's host and
reduce a compromise of that host to a replaceable serving key.  This document
does not specify where a broker root key is held.

Depth is bounded at one link rather than left to a path builder.  A general
path builder brings certificate policies, name constraints and revocation,
none of which this protocol uses, and each of which is a way for a chain to
be accepted that the pin did not authorise.  One signature under one pinned
key admits nothing the pin did not.

No agent evaluates a validity period, and a certificate carrying a broker
root key carries the notAfter value that Section 4.1.2.5 of
[RFC5280][rfc5280] reserves for a certificate with no well-defined
expiration date.  A validity period an agent enforced would be a scheduled
outage: at its end every agent refuses the broker, no announcement can
prevent that, and recovery is the re-provisioning this design exists to
avoid.  Enforcing one would also place each agent's clock in the path of
connecting, which fails a host restored from an old snapshot, a host with a
dead real-time clock, and a first boot before time is synchronised.
[IEEE802.1AR][ieee8021ar] reaches the same conclusion for a device identity
installed at manufacture, giving it the same notAfter value for the same
reason: nobody can reach the devices that hold it.

Bounding the exposure of a stolen serving key is left to the frequency of
replacement rather than to a validity period, which no agent reads.

### C.16. The Trusted Consumer

The consumer sits inside the broker's trust boundary because the protocol
cannot place it elsewhere without specifying a second authentication system:
consumers do not hold PTP identities, and the broker environment that admits
them already has one.  What the protocol can do is state the boundary, which
[Section 12.3][s12.3] does, so that the trust is a decision a deployment
reads rather than an assumption it discovers.  The weight of the decision
grows with reach: under PTP/1 a consumer reaches single hosts, at the
selectors each host's own [POL-8][pol-8] serves, and a future version whose
designators front whole networks re-examines the boundary before it ships.

### C.17. One Broker Process

The continuity design leans on facts that are true of a single process: a
restart ends every connection, and compare-and-record is atomic against one
store.  Stating the assumption ([STM-18][stm-18]) costs a sentence; leaving
it unstated invites the deployment it breaks, because a load balancer in
front of two broker processes looks like an availability improvement and
silently disables [SEC-6][sec-6] ([Section 12.5][s12.5]).  Distributing a
broker is a real design with real costs: a linearisable shared record store,
and per-identifier ownership so that displacement can end a connection held
elsewhere.  A document that has not done that work says so rather than
implying the work is free.

### C.18. Reconnection Backoff

The exemptions of [STM-14][stm-14] name three releases rather than exempting
every connection the agent released itself.  The broader exemption opens a
loop: an agent aborting on `ACCESS`(Denied) releases the connection itself,
so a frozen or withdrawn identifier, or a fleet behind a broker whose
records were reset, could reconnect in a tight loop with full conformance,
each iteration durably spending a sequence ([STM-15][stm-15]) and costing
the broker an admission attempt.  The bound doubles on consecutive failures
because the agent cannot learn why it was denied ([SEC-7][sec-7]): a
condition it cannot observe ending is one it must poll, and polling
something that may never change is done at a rate that decays to a floor.
An overtaken connection cannot form that loop.  It ends only because another
connection attempt of the same race was selected ([Section 6.3][s6.3]), so a
race overtakes fewer connections than it has connection attempts, and none
of them carries a claim ([Section 6.3][s6.3]).  A failed race doubles the
bound as a denial does.  A broker that is down or cannot be reached is a
condition the agent cannot observe ending either, and without the doubling
an agent would start races against it without pause.

The bound starts at 5 seconds and doubles to at most 60 seconds, so every
delay is at least 5 seconds and less than 120 seconds.  The start bounds the
work a broker restart causes: a fleet whose stable connections end together
reaches the broker spread over the interval from 5 to 10 seconds after those
ends, one admission attempt per agent.  The ceiling bounds recovery: after
an outage of any length, the delay pending when the network returns is less
than 120 seconds.  A denied agent polls at the same ceiling, at most one
admission attempt per 60 seconds.

The bound is independent of K.  K is chosen for liveness ([Appendix
C.4][sC.4]), and an agent on a metered link that lengthens K would otherwise
recover more slowly from every outage.

The bound returns to 5 seconds only after a connection has remained in
SERVING for K.  Two holders of one keypair each displace the other on
admission ([STM-11][stm-11]), and each of their connections reaches SERVING.
A bound that returned on reaching SERVING would let the pair alternate every
5 to 10 seconds without end.  A connection displaced before K counts as a
failure, so the pair's bounds grow as those of any failure do.

### C.19. Binary Claims

A printable claim encoding, carried in a text field, obliges a document to a
grammar, case rules for hexadecimal identifiers, leading-zero prohibitions
and digit bounds, all so that one value has one encoding wherever it is
compared.  A frame of fixed-width binary fields ([Section 2.4][s2.4]) has
one encoding by construction, and the class of defect disappears with the
prose that would manage it.

### C.20. Ranges over Ports, a Wildcard over Paths

A selector type that names one service announces any set a host can
enumerate, and cannot announce every port: 65535 `tcp-port` selectors encode
to some 196000 octets against the 65535 a payload holds ([Section
2.4][s2.4]).  An agent serving every port would otherwise announce the empty
set, and [STM-13][stm-13] would report to the operator the opposite of what
that agent serves.  A range answers this in four octets, and answers the
narrower case, a block of ports, in the same four.

The ranges are one type per port type rather than one type spanning both, so
that an agent can serve every TCP port and no UDP port.

Paths take a wildcard, `unix-path-any`, rather than a range, because a range
is bounded by an ordering and paths carry none that names a predictable set
([Section 10.3][s10.3]).  A wildcard is also the whole of what the path case
needs: an operator either enumerates the sockets, which a set already
carries, or declines to, which is one bit.

Neither a range nor a wildcard grants anything.  The announced set is
informational ([Section 3.1][s3.1]), so an agent announcing every port has
promised none of them, and [POL-8][pol-8] answers each stream as it arrives.
What a deployment grants by answering every selector of a type is examined
in [Section 12.9][s12.9].

### C.21. A Counter, Not a Clock

Replacing a broker serving key takes nothing from a party holding the one it
replaced.  The certificate that party holds stays signed by the broker root
key, and [Appendix C.15][sC.15] leaves an agent nothing else to read.  Two
certificates signed by the same pinned key are therefore indistinguishable
to the agent that must choose between them, and the agent needs a value that
orders them.

A validity period is the value most deployments use, and this document
rejects it.  A certificate lifetime must exceed the largest clock error the
deployment tolerates, or a correct agent is refused; so a lifetime short
enough to bound the exposure is a lifetime that requires synchronised clocks
on every agent.  An agent is reached only through the tunnel it establishes,
so an agent that refuses the handshake is an agent nobody can reach to fix.
Every failure of a counter is repaired at the broker, and every failure of a
validity period is repaired at each agent.

A monotonic counter is the established answer where clients cannot be
assumed to hold the correct time.  [TUF][tuf] refuses metadata whose version
number is lower than the version already held, and names the attack it
prevents a rollback attack.  [UPTANE][uptane], which specifies over-the-air
updates for vehicle electronic control units, states that such units have no
reliable real-time clock and refuses both a lower release counter and an
earlier time than the one already recorded.  Android Verified Boot carries a
rollback index in each image and refuses to boot one lower than the index in
tamper-evident storage.  WireGuard keeps the greatest timestamp it has
accepted from each peer and discards an initiation carrying one no greater,
which requires no synchronised clock on the receiving side.

The counter is carried as the X.509 serialNumber rather than as an
extension.  The field is present in every certificate, it is an integer, and
a party that does not implement this document reads it as the serial number
it is.

The width is bounded at eight octets, where Section 4.1.2.2 of
[RFC5280][rfc5280] permits twenty.  The bound keeps the comparison an
agent performs on every handshake within one machine word, so an arbitrary
number arriving from the network is never compared as a bignum.  A broker
that left the field to a certificate generator would commonly find a hash
there, twenty octets wide, and an agent refuses that rather than reducing it
to a number the generator did not mean.  Eight octets carry 2^64 replacements
of a serving key, which no deployment reaches.

A rollback index is not a protocol version.  The wire is unchanged, an agent
refusing an index aborts at the TLS layer before transmitting the version
octet of [TRN-11][trn-11], and [VER-3][ver-3] permits a retry only after a
verified broker denies one.  A version bump would therefore reach no agent
that this rule refuses.  What an operator needs before replacing a serving
key is not a version but a count of the agents whose software applies
[TRN-2][trn-2]'s third verification at all, and [VER-4][ver-4] already
obliges a broker to make the declared version of a denied connection
visible.

### C.22. Racing Connection Attempts

A connection attempt that receives no answer fails only when the agent stops
waiting for it, and a network that discards UDP without an error makes every
QUIC attempt fail that way.  Made in sequence, the connection attempts of
[Section 6.3][s6.3] spend one full wait for each QUIC address before the
first TCP attempt starts, at every reconnection.  Starting each attempt 250
milliseconds after the one before it bounds the wait for the first attempt
that can succeed at 250 milliseconds for each attempt ahead of it.  An
attempt in progress is not ended when the next starts, so on a link where a
handshake takes longer than 250 milliseconds the attempts listed first still
complete.  The value and the alternation of address families are those of
[RFC8305][rfc8305], which applies the same method to IPv6 and IPv4
addresses.

The race is mandatory.  An agent is reached only through its tunnel, so an
agent that waits out a timeout before it falls back is unreachable for the
whole of that wait, at every reconnection.

The cost lands at the broker.  An attempt ended after its handshake started
costs the broker a handshake that is then ended, and the bound of
[TRN-10][trn-10] on unadmitted connections counts that attempt until it
ends.  An overtaken connection leaves the reconnection bound unchanged
([STM-14][stm-14]), because it ends only when another attempt of the same
race has been selected.  Only the selected attempt carries a protocol
message, so a race presents one claim.  Which attempt the agent selects
among those that completed [TRN-2][trn-2] is immaterial, the two mappings
being equivalent ([Appendix C.6][sC.6]).  A second `CLAIM` would spend a
sequence ([STM-15][stm-15]) and displace the first connection
([STM-11][stm-11]).

The race leaves [Section 12.7][s12.7] unchanged.  A party able to delay the
QUIC handshake by more than 250 milliseconds selects the mapping of
[Appendix B][appendix-b], as a party that blocks UDP already can, and the
two mappings remain equivalent.

---

## Appendix D. Summary of Labels (Informative)

Every label defined in this document, with the section defining it.  This
appendix restates; it adds nothing.  Where it disagrees with the defining
section, the defining section governs.

### D.1. Transport Requirements

A transport implementation satisfies these.

| Label | Name | Defined in |
|-------|------|------------|
| [TRN-1][trn-1] | Establishment | [Section 7][s7] |
| [TRN-2][trn-2] | Broker authentication | [Section 7][s7] |
| [TRN-3][trn-3] | Agent key proof | [Section 7][s7] |
| [TRN-4][trn-4] | Confidentiality and integrity | [Section 7][s7] |
| [TRN-5][trn-5] | Credential handling | [Section 7][s7] |
| [TRN-6][trn-6] | Streams | [Section 7][s7] |
| [TRN-7][trn-7] | Ordering | [Section 7][s7] |
| [TRN-8][trn-8] | Liveness | [Section 7][s7] |
| [TRN-9][trn-9] | Default deny | [Section 7][s7] |
| [TRN-10][trn-10] | Limits | [Section 7][s7] |
| [TRN-11][trn-11] | Version | [Section 7][s7] |
| [TRN-12][trn-12] | Demultiplexing | [Section 7][s7] |
| [TRN-13][trn-13] | Serving key index | [Section 7][s7] |

### D.2. Policy Requirements

A policy implementation satisfies these.

| Label | Name | Defined in |
|-------|------|------------|
| [POL-1][pol-1] | Access | [Section 8.1][s8.1] |
| [POL-2][pol-2] | Join | [Section 8.1][s8.1] |
| [POL-3][pol-3] | Register | [Section 8.1][s8.1] |
| [POL-4][pol-4] | Issuance | [Section 8.1][s8.1] |
| [POL-5][pol-5] | Withdrawal | [Section 8.1][s8.1] |
| [POL-6][pol-6] | Continuity | [Section 8.1][s8.1] |
| [POL-7][pol-7] | Broker pin | [Section 8.2][s8.2] |
| [POL-8][pol-8] | Target | [Section 8.2][s8.2] |
| [POL-9][pol-9] | Agent identity | [Section 8.2][s8.2] |
| [POL-10][pol-10] | Sequence | [Section 8.2][s8.2] |
| [POL-11][pol-11] | Rollback index | [Section 8.2][s8.2] |

### D.3. State Machine Requirements

A protocol implementation satisfies these.

| Label | Name | Defined in |
|-------|------|------------|
| [STM-1][stm-1] | Single driver | [Section 9][s9] |
| [STM-2][stm-2] | Responder state | [Section 9][s9] |
| [STM-3][stm-3] | Protocol errors | [Section 9][s9] |
| [STM-4][stm-4] | Stream availability | [Section 9][s9] |
| [STM-5][stm-5] | Release | [Section 9][s9] |
| [STM-6][stm-6] | Token confinement | [Section 9][s9] |
| [STM-7][stm-7] | Join idempotence | [Section 9][s9] |
| [STM-8][stm-8] | Error classification | [Section 9][s9] |
| [STM-9][stm-9] | No embedded judgement | [Section 9][s9] |
| [STM-10][stm-10] | One claim per connection | [Section 9][s9] |
| [STM-11][stm-11] | Admission replacement | [Section 9][s9] |
| [STM-12][stm-12] | Replacement visibility | [Section 9][s9] |
| [STM-13][stm-13] | Announcement visibility | [Section 9][s9] |
| [STM-14][stm-14] | Reconnection | [Section 9][s9] |
| [STM-15][stm-15] | Sequence durability | [Section 9][s9] |
| [STM-16][stm-16] | Freeze visibility | [Section 9][s9] |
| [STM-17][stm-17] | No inbound surface | [Section 9][s9] |
| [STM-18][stm-18] | Broker locality | [Section 9][s9] |
| [STM-19][stm-19] | Stream admission | [Section 4.3][s4.3] |
| [STM-20][stm-20] | Accept ordering | [Section 4.3][s4.3] |
| [STM-21][stm-21] | Freeze failure visibility | [Section 9][s9] |

### D.4. Versioning Requirements

Binding on brokers and agents alike.

| Label | Name | Defined in |
|-------|------|------------|
| [VER-1][ver-1] | Declaration | [Section 10.1][s10.1] |
| [VER-2][ver-2] | Supported range | [Section 10.1][s10.1] |
| [VER-3][ver-3] | Authenticated retry | [Section 10.1][s10.1] |
| [VER-4][ver-4] | Operator visibility | [Section 10.1][s10.1] |
| [VER-5][ver-5] | Mapping scope | [Section 10.1][s10.1] |
| [VER-6][ver-6] | Deployment minimum | [Section 10.1][s10.1] |
| [VER-7][ver-7] | Version invariants | [Section 10.2][s10.2] |

### D.5. Conformance Tests

An implementation claiming conformance passes these, once per mapping it
claims.

| Label | Name | Defined in |
|-------|------|------------|
| [CNF-1][cnf-1] | Stream reaches service | [Section 11][s11] |
| [CNF-2][cnf-2] | Unserved selector refused | [Section 11][s11] |
| [CNF-3][cnf-3] | Join binding | [Section 11][s11] |
| [CNF-4][cnf-4] | Denial without registration | [Section 11][s11] |
| [CNF-5][cnf-5] | Unpinned broker aborted | [Section 11][s11] |
| [CNF-6][cnf-6] | Registration replacement | [Section 11][s11] |
| [CNF-7][cnf-7] | Idle connection survives | [Section 11][s11] |
| [CNF-8][cnf-8] | Silent peer deregistered | [Section 11][s11] |
| [CNF-9][cnf-9] | Unsupported version denied | [Section 11][s11] |
| [CNF-10][cnf-10] | Malformed frame ends connection | [Section 11][s11] |
| [CNF-11][cnf-11] | Withdrawal restores joining | [Section 11][s11] |
| [CNF-12][cnf-12] | Denial is uniform | [Section 11][s11] |
| [CNF-13][cnf-13] | Stale sequence freezes | [Section 11][s11] |
| [CNF-14][cnf-14] | Sequence needs the key first | [Section 11][s11] |
| [CNF-15][cnf-15] | Admission displaces connection | [Section 11][s11] |
| [CNF-16][cnf-16] | Concurrent streams | [Section 11][s11] |
| [CNF-17][cnf-17] | Datagram boundaries preserved | [Section 11][s11] |
| [CNF-18][cnf-18] | Socket path reaches service | [Section 11][s11] |
| [CNF-19][cnf-19] | Denied agent delays reconnection | [Section 11][s11] |
| [CNF-20][cnf-20] | Announcement decides nothing | [Section 11][s11] |
| [CNF-21][cnf-21] | Forwards past the ceiling | [Section 11][s11] |
| [CNF-22][cnf-22] | Joining recovers a frozen identifier | [Section 11][s11] |
| [CNF-23][cnf-23] | An announced set is not dialable | [Section 11][s11] |
| [CNF-24][cnf-24] | Uncertified serving key aborted | [Section 11][s11] |
| [CNF-25][cnf-25] | Superseded serving key aborted | [Section 11][s11] |
| [CNF-26][cnf-26] | Unchanged index admitted | [Section 11][s11] |
| [CNF-27][cnf-27] | Nothing accepted admits any index | [Section 11][s11] |
| [CNF-28][cnf-28] | Oversized index aborted | [Section 11][s11] |

### D.6. Security Properties

Provided where the above are met; not separately claimed.

| Label | Name | Defined in |
|-------|------|------------|
| [SEC-1][sec-1] | Mutual authentication | [Section 12.1][s12.1] |
| [SEC-2][sec-2] | No listening surface | [Section 12.1][s12.1] |
| [SEC-3][sec-3] | Join integrity | [Section 12.1][s12.1] |
| [SEC-4][sec-4] | Single-use bootstrap | [Section 12.1][s12.1] |
| [SEC-5][sec-5] | Least reach | [Section 12.1][s12.1] |
| [SEC-6][sec-6] | Detectable compromise | [Section 12.1][s12.1] |
| [SEC-7][sec-7] | No record disclosure | [Section 12.1][s12.1] |

<!-- section links -->

[s1]: #1-introduction
[s1.1]: #11-requirements-language
[s1.2]: #12-terminology
[s1.3]: #13-protocol-surfaces
[s2]: #2-encodings
[s2.1]: #21-identifiers-and-integers
[s2.2]: #22-public-key-encoding
[s2.3]: #23-selector-encoding
[s2.4]: #24-control-stream-and-frames
[s2.5]: #25-tunnel-streams
[s3]: #3-message-vocabulary
[s3.1]: #31-messages
[s3.2]: #32-access-verdicts
[s4]: #4-protocol-state-machines
[s4.1]: #41-agent-state-machine
[s4.2]: #42-broker-state-machine
[s4.3]: #43-stream-sub-protocol
[s5]: #5-procedures
[s5.1]: #51-serving
[s5.2]: #52-joining
[s6]: #6-connection-management
[s6.1]: #61-registration
[s6.2]: #62-liveness
[s6.3]: #63-transport-selection
[s7]: #7-transport-requirements
[s8]: #8-policy-requirements
[s8.1]: #81-broker-policy
[s8.2]: #82-agent-policy
[s9]: #9-state-machine-requirements
[s10]: #10-versioning
[s10.1]: #101-version-rules
[s10.2]: #102-version-invariant-rules
[s10.3]: #103-reserved-extensions
[s11]: #11-conformance
[s12]: #12-security-considerations
[s12.1]: #121-properties
[s12.2]: #122-threat-analysis
[s12.3]: #123-the-consumer
[s12.4]: #124-fixed-identities
[s12.5]: #125-broker-locality
[s12.6]: #126-version-downgrade
[s12.7]: #127-mapping-downgrade
[s12.8]: #128-timing-of-denials
[s12.9]: #129-unenumerated-announcements
[s13]: #13-iana-considerations
[s14]: #14-references
[s14.1]: #141-normative-references
[s14.2]: #142-informative-references
[appendix-a]: #appendix-a-quic-transport-mapping-normative
[appendix-b]: #appendix-b-tls-over-tcp-transport-mapping-normative
[appendix-c]: #appendix-c-design-rationale-informative
[sC.1]: #c1-two-verdicts
[sC.2]: #c2-idempotent-joining
[sC.3]: #c3-replacement-rather-than-rejection
[sC.4]: #c4-liveness-parameters-are-local
[sC.5]: #c5-absence-of-should
[sC.6]: #c6-one-wire-protocol-two-mappings
[sC.7]: #c7-the-choice-of-yamux
[sC.8]: #c8-datagrams-over-streams
[sC.9]: #c9-a-stream-proxy-not-a-packet-tunnel
[sC.10]: #c10-withdrawal-as-a-separate-operation
[sC.11]: #c11-freezing-rather-than-refusing
[sC.12]: #c12-bounding-admitted-connections
[sC.13]: #c13-bounding-who-holds-admission
[sC.14]: #c14-detection-without-overlap
[sC.15]: #c15-fixed-identities
[sC.16]: #c16-the-trusted-consumer
[sC.17]: #c17-one-broker-process
[sC.18]: #c18-reconnection-backoff
[sC.19]: #c19-binary-claims
[sC.20]: #c20-ranges-over-ports-a-wildcard-over-paths
[sC.21]: #c21-a-counter-not-a-clock
[sC.22]: #c22-racing-connection-attempts
[appendix-d]: #appendix-d-summary-of-labels-informative
[sD.1]: #d1-transport-requirements
[sD.2]: #d2-policy-requirements
[sD.3]: #d3-state-machine-requirements
[sD.4]: #d4-versioning-requirements
[sD.5]: #d5-conformance-tests
[sD.6]: #d6-security-properties

<!-- requirement labels -->

[trn-1]: #trn-1
[trn-2]: #trn-2
[trn-3]: #trn-3
[trn-4]: #trn-4
[trn-5]: #trn-5
[trn-6]: #trn-6
[trn-7]: #trn-7
[trn-8]: #trn-8
[trn-9]: #trn-9
[trn-10]: #trn-10
[trn-11]: #trn-11
[trn-12]: #trn-12
[trn-13]: #trn-13
[pol-1]: #pol-1
[pol-2]: #pol-2
[pol-3]: #pol-3
[pol-4]: #pol-4
[pol-5]: #pol-5
[pol-6]: #pol-6
[pol-7]: #pol-7
[pol-8]: #pol-8
[pol-9]: #pol-9
[pol-10]: #pol-10
[pol-11]: #pol-11
[stm-1]: #stm-1
[stm-2]: #stm-2
[stm-3]: #stm-3
[stm-4]: #stm-4
[stm-5]: #stm-5
[stm-6]: #stm-6
[stm-7]: #stm-7
[stm-8]: #stm-8
[stm-9]: #stm-9
[stm-10]: #stm-10
[stm-11]: #stm-11
[stm-12]: #stm-12
[stm-13]: #stm-13
[stm-14]: #stm-14
[stm-15]: #stm-15
[stm-16]: #stm-16
[stm-17]: #stm-17
[stm-18]: #stm-18
[stm-19]: #stm-19
[stm-20]: #stm-20
[stm-21]: #stm-21
[ver-1]: #ver-1
[ver-2]: #ver-2
[ver-3]: #ver-3
[ver-4]: #ver-4
[ver-5]: #ver-5
[ver-6]: #ver-6
[ver-7]: #ver-7
[cnf-1]: #cnf-1
[cnf-2]: #cnf-2
[cnf-3]: #cnf-3
[cnf-4]: #cnf-4
[cnf-5]: #cnf-5
[cnf-6]: #cnf-6
[cnf-7]: #cnf-7
[cnf-8]: #cnf-8
[cnf-9]: #cnf-9
[cnf-10]: #cnf-10
[cnf-11]: #cnf-11
[cnf-12]: #cnf-12
[cnf-13]: #cnf-13
[cnf-14]: #cnf-14
[cnf-15]: #cnf-15
[cnf-16]: #cnf-16
[cnf-17]: #cnf-17
[cnf-18]: #cnf-18
[cnf-19]: #cnf-19
[cnf-20]: #cnf-20
[cnf-21]: #cnf-21
[cnf-22]: #cnf-22
[cnf-23]: #cnf-23
[cnf-24]: #cnf-24
[cnf-25]: #cnf-25
[cnf-26]: #cnf-26
[cnf-27]: #cnf-27
[cnf-28]: #cnf-28
[sec-1]: #sec-1
[sec-2]: #sec-2
[sec-3]: #sec-3
[sec-4]: #sec-4
[sec-5]: #sec-5
[sec-6]: #sec-6
[sec-7]: #sec-7

<!-- normative and informative references -->

[rfc2104]: https://www.rfc-editor.org/rfc/rfc2104
[rfc2119]: https://www.rfc-editor.org/rfc/rfc2119
[ieee8021ar]: https://standards.ieee.org/standard/802_1AR-2018.html
[rfc4787]: https://www.rfc-editor.org/rfc/rfc4787
[rfc5280]: https://www.rfc-editor.org/rfc/rfc5280
[rfc5382]: https://www.rfc-editor.org/rfc/rfc5382
[rfc7030]: https://www.rfc-editor.org/rfc/rfc7030
[rfc6234]: https://www.rfc-editor.org/rfc/rfc6234
[rfc7301]: https://www.rfc-editor.org/rfc/rfc7301
[rfc8174]: https://www.rfc-editor.org/rfc/rfc8174
[rfc8305]: https://www.rfc-editor.org/rfc/rfc8305
[rfc8446]: https://www.rfc-editor.org/rfc/rfc8446
[rfc8555]: https://www.rfc-editor.org/rfc/rfc8555
[rfc9000]: https://www.rfc-editor.org/rfc/rfc9000
[rfc9001]: https://www.rfc-editor.org/rfc/rfc9001
[rfc9113]: https://www.rfc-editor.org/rfc/rfc9113
[rfc9221]: https://www.rfc-editor.org/rfc/rfc9221
[rfc9562]: https://www.rfc-editor.org/rfc/rfc9562
[tuf]: https://theupdateframework.github.io/specification/latest/
[uptane]: https://uptane.org/docs/2.1.0/standard/uptane-standard
[yamux]: https://github.com/hashicorp/yamux/blob/master/spec.md
