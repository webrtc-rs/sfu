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
provides one such I/O layer (an HTTP signaling server + a UDP media socket).

### Sans-IO core

`Sfu` is the public entry point and implements
[`sansio::Protocol<TaggedBytesMut, Infallible, Event>`](https://docs.rs/sansio):

| Method                                                           | Plane     | Meaning                                                                            |
|------------------------------------------------------------------|-----------|------------------------------------------------------------------------------------|
| `handle_read(TaggedBytesMut)` / `poll_write() -> TaggedBytesMut` | media     | push an incoming UDP datagram in / drain an outgoing datagram                      |
| `handle_event(Event)` / `poll_event() -> Event`                  | signaling | push a signaling request in / drain a signaling response or server-initiated event |
| `handle_timeout(Instant)` / `poll_timeout() -> Instant`          | clock     | advance the caller-supplied clock / ask for the next deadline                      |

The read/write planes carry raw datagrams (`Rin = Wout = TaggedBytesMut`): an inbound
datagram is demultiplexed to a client and fed to that client's `RTCPeerConnection`, and
the datagrams every client produces are drained back out. The application plane is
unused (`Rout = Win = Infallible`) — there is no payload *above* the SFU; inbound media
is meant to be forwarded *internally* between clients (the fan-out is still being wired
up — see the TODO note below), never surfaced to the caller. **All signaling is the
event plane** (`Ein = Eout = Event`).

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

- **`Event`** ([`src/event.rs`](src/event.rs)) is the unified signaling currency:
  `Join`, `SessionDescription`, `IceCandidate`, `Leave`, plus the `Ok`/`Err`
  replies. Every variant carries a `request_id`, `room_id`, and `client_id`, which is
  how `Sfu` routes it to the right `Room` and `Client`.
- **`Demuxer`** ([`src/demuxer.rs`](src/demuxer.rs)) maps an incoming datagram to a
  `(RoomId, ClientId)`: by learned 4-tuple affinity once media flows, and otherwise by
  parsing the STUN `USERNAME` local-ufrag during ICE. Both `Sfu` and `Room` hold one —
  `Sfu` demuxes to the room, `Room` demuxes to the client.
- **`ForwardTable`** ([`src/forward.rs`](src/forward.rs)) is the (planned) media routing
  graph — publisher track → the set of subscriber senders it is forwarded to. It is
  defined but not yet owned by `Room`; wiring it into the RTP fan-out is still TODO.

**Routing over one shared socket (ICE-lite).** Because every client is multiplexed over
the same UDP socket, the SFU has to tell datagrams apart. Each client is built in
**ICE-lite** mode with a *local ufrag that encodes its address*: `"{room_id}/{client_id}
+{random}"`. A browser's STUN binding request carries `USERNAME = local_ufrag:remote_ufrag`,
so the `Demuxer` recovers `(room_id, client_id)` straight from the local ufrag, then
caches the 4-tuple for the DTLS/SRTP phase. The SFU's answer also advertises a host ICE
candidate synthesized from `local_addr`.

For example, an `Event::Join` creates the `Room` and the `Client` (default media engine

+ codecs, default interceptor chain, and a setting engine carrying the ICE-lite creds
  above); an `Event::SessionDescription` offer is answered by the client
  (`set_remote_description` → add the `local_addr` host candidate → `create_answer` →
  `set_local_description`) and the resulting **answer** is emitted back out through
  `poll_event`; an `Event::Leave` tears the client down and reaps the room once empty.

## Repository Layout

```text
sfu/
├── Cargo.toml            # root sfu crate manifest
├── src/
│   ├── lib.rs            # crate root + public re-exports (Sfu, Event, Room/Client ids, …)
│   ├── sfu.rs            # Sfu: top-level sans-IO Protocol; owns rooms + Demuxer + local_addr
│   ├── room.rs           # Room: owns clients + a Demuxer
│   ├── client.rs         # Client: wraps one rtc RTCPeerConnection (PeerConnection trait)
│   ├── event.rs          # Event enum (signaling currency) + RequestId
│   ├── demuxer.rs        # Demuxer: STUN ufrag + 4-tuple -> (RoomId, ClientId)
│   └── forward.rs        # ForwardTable: publisher track -> subscriber senders (not yet wired in)
├── examples/
│   ├── chat.rs           # runnable SFU server: HTTP(S) signaling + UDP media
│   ├── chat.html         # browser test client
│   ├── signaling/        # example HTTP <-> Event glue
│   └── util/             # example helpers + self-signed cert/key
├── apprtc/               # submodule: AppRTC HTTP front-door crate
├── signaling/            # submodule: WebSocket signaling hub crate
├── rtc/                  # submodule: sans-IO WebRTC core (rtc)
├── scripts/              # helper scripts
└── todo_tests/           # migration/reference tests and examples
```

The public API surface of the crate is `Sfu` / `SfuId`, `RoomId`, `ClientId`, and
`Event` / `RequestId`.

## Building

### Toolchain

Use a Rust toolchain with Edition 2024 support.

### Build & test

```bash
# Update rtc submodule first
git submodule update --init --recursive
cargo clippy
cargo fmt
cargo build
cargo test           # unit tests for Sfu (join/leave/offer→answer), Client, and Demuxer
```

### Run the `chat` example

The `chat` example wraps the sans-IO core with real I/O: a `rouille` signaling server
and one UDP media socket per media port. Every WebRTC flow is multiplexed over the
media socket(s); clients are identified by their remote address (and STUN ufrag).

```bash
# HTTPS (self-signed cert in examples/util), the default
cargo run --example chat

# plain HTTP, e.g. for local integration testing
cargo run --example chat -- --force-local-loop
```

Then open the printed URL in a browser (it serves `examples/chat.html`). Useful flags:
`--host` (default `127.0.0.1`), `--signal-port` (default `8080`),
`--media-port-min`/`--media-port-max` (default `3478`–`3495`), `--debug`, and
`--level <error|warn|info|debug|trace>`.

## Notes for Contributors

- The active implementation is the root `sfu` library crate; keep it sans-IO —
  no sockets, threads, or `Instant::now()` inside `src/`. All real I/O belongs in
  examples (or downstream binaries).
- `apprtc/`, `signaling/`, and `rtc/` are git submodules consumed as path
  dependencies: `apprtc` is the HTTP front door, `signaling` is the WebSocket hub, and
  `rtc` is the underlying sans-IO WebRTC stack.
- The architecture is landing in phases. Signaling and datagram routing are wired
  end-to-end (join → offer/answer, `handle_read` demux → PC, `poll_write` drain), but a
  few pieces are still TODO in `src/`: forwarding inbound RTP to subscribers
  (`Client::poll_read` → `ForwardTable` fan-out), turning `RTCPeerConnection` events
  (`OnTrack`, ICE candidates) into `Event`s / `ForwardTable` updates instead of logging
  them, and `Demuxer` affinity expiry/eviction.

## Open Source License

This project uses dual licensing under MIT or Apache-2.0.

## Contributing

Contributors and pull requests are welcome.
