<h1 align="center">
 <a href="https://sfu.rs"><img src="https://raw.githubusercontent.com/webrtc-rs/webrtc-rs.github.io/master/res/sfu-rs.png" alt="SFU"></a>
 <br>
</h1>
<p align="center">
 <a href="https://github.com/webrtc-rs/sfu/actions">
  <img src="https://github.com/webrtc-rs/sfu/workflows/cargo/badge.svg">
 </a>
 <a href="https://codecov.io/gh/webrtc-rs/sfu">
  <img src="https://codecov.io/gh/webrtc-rs/sfu/branch/master/graph/badge.svg">
 </a>
 <a href="https://deps.rs/repo/github/webrtc-rs/sfu">
  <img src="https://deps.rs/repo/github/webrtc-rs/sfu/status.svg">
 </a>
 <a href="https://crates.io/crates/sfu">
  <img src="https://img.shields.io/crates/v/sfu.svg">
 </a>
 <a href="https://docs.rs/sfu">
  <img src="https://docs.rs/sfu/badge.svg">
 </a>
 <a href="https://doc.rust-lang.org/1.6.0/complement-project-faq.html#why-dual-mitasl2-license">
  <img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue" alt="License: MIT/Apache 2.0">
 </a>
 <a href="https://discord.gg/4Ju8UHdXMs">
  <img src="https://img.shields.io/discord/800204819540869120?logo=discord" alt="Discord">
 </a>
 <a href="https://twitter.com/WebRTCrs">
  <img src="https://img.shields.io/twitter/url/https/twitter.com/webrtcrs.svg?style=social&label=%40WebRTCrs" alt="Twitter">
 </a>
</p>
<p align="center">
 SFU in Rust with Sans-IO
</p>

## Overview

`sfu` is a **Selective Forwarding Unit** built on the modern **sans-IO** `rtc` core.
The library crate contains **no sockets, no threads, and no clock of its own** — it is
a pure state machine. The caller owns all I/O: it feeds datagrams and signaling in,
and drains datagrams and signaling out. The bundled [`chat`](examples/chat.rs) example
provides one such I/O layer (an HTTPS / WebSocket signaling server + a UDP media socket).

### Sans-IO core

`Sfu` is the public entry point and implements
[`sansio::Protocol<TaggedBytesMut, Infallible, SFUEvent>`](https://docs.rs/sansio):

| Method                                                           | Plane     | Meaning                                                                            |
|------------------------------------------------------------------|-----------|------------------------------------------------------------------------------------|
| `handle_read(TaggedBytesMut)` / `poll_write() -> TaggedBytesMut` | media     | push an incoming UDP datagram in / drain an outgoing datagram                      |
| `handle_event(SFUEvent)` / `poll_event() -> SFUEvent`            | signaling | push a signaling request in / drain a signaling response or server-initiated event |
| `handle_timeout(Instant)` / `poll_timeout() -> Instant`          | clock     | advance the caller-supplied clock / ask for the next deadline                      |

The read/write planes carry raw datagrams (`Rin = Wout = TaggedBytesMut`): an inbound
datagram is demultiplexed to a client and fed to that client's `RTCPeerConnection`, and
the datagrams every client produces are drained back out. The application plane is
unused (`Rout = Win = Infallible`) — there is no payload *above* the SFU; inbound media
(RTP) and keyframe requests (RTCP PLI/FIR) are forwarded *internally* between clients via
`ForwardTable`, never surfaced to the caller. **All signaling is the event plane**
(`Ein = Eout = SFUEvent`).

The engine is constructed with the local media address it will advertise in answers:
`Sfu::new(id, local_addr)`.

### Architecture — three nested sans-IO layers

`Sfu`, `Room`, and `Client` each implement the same `Protocol`, so signaling is simply
routed down and events bubble back up:

```
Sfu   ── owns rooms: HashMap<RoomId, Room>, a Demuxer, the local_addr, transmits/events
 └─ Room   ── owns clients: HashMap<ClientId, Client> and its own Demuxer
     └─ Client ── wraps exactly one rtc RTCPeerConnection (via the PeerConnection trait)
```

- **`SFUEvent`** ([`src/event.rs`](src/event.rs)) is the unified signaling currency:
  `Join`, `SessionDescription`, `IceCandidate`, `Leave`, plus the `Ok`/`Err`
  replies. Every variant carries a `request_id`, `room_id`, and `client_id`, which is
  how `Sfu` routes it to the right `Room` and `Client`.
- **`Demuxer`** ([`src/demuxer.rs`](src/demuxer.rs)) maps an incoming datagram to a
  `(RoomId, ClientId)`: by learned 4-tuple affinity once media flows, and otherwise by
  parsing the STUN `USERNAME` local-ufrag during ICE. Both `Sfu` and `Room` hold one —
  `Sfu` demuxes to the room, `Room` demuxes to the client.
- **`ForwardTable`** ([`src/forward.rs`](src/forward.rs)) is the media routing graph, owned
  by `Room`. It is keyed by `ForwardKey { publisher, mid }` — the publisher's m-line **mid**,
  not the SSRC, is the stable dedup key across renegotiations — and maps each publishing
  track to the subscriber senders it fans out to, with an SSRC index for routing inbound RTP.
  `Room::reconcile` diffs the desired forwarding matrix whenever membership or published
  tracks change, adding a sendonly transceiver per new subscriber and tearing down stale ones.
- **`RtcpForwarderInterceptor`** ([`src/rtcp_forwarder.rs`](src/rtcp_forwarder.rs)) is installed
  as the outermost interceptor on every client so that a subscriber's keyframe requests
  (RTCP **PLI/FIR**) about a forwarded stream reach the application via `poll_read()`. `Room`
  relays them to the publisher (`Client::request_keyframe`), which is what makes a
  newly-subscribed track light up promptly.

**Routing over one shared socket (ICE-lite).** Because every client is multiplexed over
the same UDP socket, the SFU has to tell datagrams apart. Each client is built in
**ICE-lite** mode with a *local ufrag that encodes its address*: `"{room_id}/{client_id}
+{random}"`. A browser's STUN binding request carries `USERNAME = local_ufrag:remote_ufrag`,
so the `Demuxer` recovers `(room_id, client_id)` straight from the local ufrag, then
caches the 4-tuple for the DTLS/SRTP phase. The SFU's answer also advertises a host ICE
candidate synthesized from `local_addr`.

For example, an `SFUEvent::Join` creates the `Room` and the `Client` (default media engine

+ codecs, the default interceptor chain plus the `RtcpForwarder`, and a setting engine
  carrying the ICE-lite creds above); an `SFUEvent::SessionDescription` offer is answered by
  the client (`set_remote_description` → add the `local_addr` host candidate → `create_answer`
  → `set_local_description`) and the resulting **answer** is emitted back out through
  `poll_event`, while publishing a track prompts server-initiated subscribe re-offers to the
  other clients; an `SFUEvent::Leave` tears the client down, prunes its forwarding entries, and
  reaps the room once empty.

## Building

### Toolchain

Use a Rust toolchain with Edition 2024 support.

### Build & test

```bash
# Fetch the submodules first
git submodule update --init --recursive
cargo clippy
cargo fmt
cargo build
cargo test --lib     # run the unit tests in-memory (join/leave/offer→answer/forwarding)
```

### Run the `chat` example

The `chat` example wraps the sans-IO core with real I/O: a custom HTTPS / WebSocket signaling server
and one UDP media socket per media port. Every WebRTC flow is multiplexed over the
media socket(s); clients are identified by their remote address (and STUN ufrag).

```bash
# HTTPS (self-signed cert in examples/util), the default
cargo run --example chat

# run on loopback (127.0.0.1) with HTTPS, e.g. for local testing
cargo run --example chat -- --force-local-loop
```

Then open the printed URL in a browser (it serves `examples/chat.html`). Useful flags:
`--host` (default `127.0.0.1`), `--signal-port` (default `8080`),
`--media-port-min`/`--media-port-max` (default `3478`–`3495`),
`-f`/`--force-local-loop`, `--debug`, and `--level <error|warn|info|debug|trace>`.

### Integration tests

`tests/` drives the SFU end-to-end: each test is a headless WebRTC client (built on the
async [`webrtc`](webrtc/) crate) that speaks the chat server's TLS-WebSocket signaling
protocol, so a `chat` server must be running first. `data_channel_test` covers
register/connect/data-channel; `rtp_test` publishes VP8 tracks and asserts the SFU forwards
RTP, intact and in order, to every other peer in the room.

```bash
# 1. start the SFU on loopback in the background
cargo run --example chat -- -f --level info &

# 2. run the integration tests against it
cargo test --test '*'

# 3. stop the background chat server
kill $(pgrep -f "examples/chat") || true
```

CI runs the same flow in a container (see [`.github/workflows/tests.yml`](.github/workflows/tests.yml)):
it boots the `chat` server and the tests together and collects `logs/sfu.log` + `logs/test.log`.

## Notes for Contributors

- The active implementation is the root `sfu` library crate; keep it sans-IO —
  no sockets, threads, or `Instant::now()` inside `src/`. All real I/O belongs in
  examples (or downstream binaries).
- The library depends only on `rtc` (the sans-IO WebRTC stack, vendored at
  `webrtc/rtc/rtc`) and `sansio`. `apprtc/`, `signaling/`, and `webrtc/` are git submodules;
  the `chat` example carries its own TLS-WebSocket signaling in `examples/signaling/`.
- The architecture is fully operational end-to-end: signaling (join → offer/answer,
  server-initiated subscribe re-offers), datagram routing (`handle_read` demux → PC,
  `poll_write` drain), RTP fan-out via `ForwardTable`, and RTCP keyframe (PLI/FIR) relay from
  subscribers back to publishers. Remaining work includes refining `Demuxer` affinity expiry
  and eviction policies.

## Open Source License

This project uses dual licensing under MIT or Apache-2.0.

## Contributing

Contributors and pull requests are welcome.
