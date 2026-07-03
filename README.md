<h1 align="center">
 <a href="https://sfu.rs"><img src="https://raw.githubusercontent.com/webrtc-rs/sfu/master/docs/sfu-rs.png" alt="SFU"></a>
 <br>
</h1>
<p align="center">
 <a href="https://github.com/webrtc-rs/sfu/actions">
  <img src="https://github.com/webrtc-rs/sfu/workflows/cargo/badge.svg" alt="CI">
 </a>
 <a href="https://codecov.io/gh/webrtc-rs/sfu">
  <img src="https://codecov.io/gh/webrtc-rs/sfu/branch/master/graph/badge.svg" alt="Coverage">
 </a>
 <a href="https://deps.rs/repo/github/webrtc-rs/sfu">
  <img src="https://deps.rs/repo/github/webrtc-rs/sfu/status.svg" alt="Dependencies">
 </a>
 <a href="https://doc.rust-lang.org/1.6.0/complement-project-faq.html#why-dual-mitasl2-license">
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License: MIT OR Apache-2.0">
 </a>
</p>
<p align="center">
 SFU in Rust with Sans-IO
</p>

## Overview

This repository is being re-architected around the modern sans-IO `rtc` core.

The current codebase uses:

- `src/engine/` — the sans-IO SFU core boundary
- `src/driver/` — the future socket/runtime driver layer
- `apprtc/` — the Rust AppRTC signaling / Collider development crate
- `examples/sfu.rs` — the sfu runtime example

The new root crate is the active implementation target, with additional repo
artifacts such as `docs/`, `scripts/`, and `todo_tests/` still present alongside
it during the migration.

## Repository Layout

```text
sfu/
├── Cargo.toml              # root sfu package manifest
├── apprtc/                 # apprtc submodule
├── examples/
│   └── sfu.rs              # sfu runtime example
├── src/
│   ├── driver/             # driver scaffold
│   ├── engine/             # sans-IO SFU core scaffold
│   └── lib.rs              # root sfu crate
├── rtc/                    # rtc submodule
├── scripts/                # helper scripts
├── docs/                   # project documentation
└── todo_tests/             # migration/reference tests and examples
```

## Building

### Toolchain

Use a Rust toolchain with Edition 2024 support.

### Build

```bash
cargo build
```

### Format

```bash
cargo fmt
```

### Run the temporary M2 runtime

```bash
cargo run --example sfu

# Optional overrides:
#   SFU_HTTP_BIND_ADDR=127.0.0.1:8080
#   SFU_UDP_BIND_ADDR=127.0.0.1:3478
#   SFU_UDP_CANDIDATE_ADDR=127.0.0.1:3478
```

The temporary M2 signaling path accepts a raw `RTCSessionDescription` JSON offer at
`POST /offer` and returns the JSON answer body. The example currently drives one
client end-to-end on a single UDP socket for bring-up, while the library crate
stays free of runtime/HTTP dependencies.

## Notes for Contributors

- The active implementation now lives under the root `sfu` crate.
- The `apprtc/` directory is a separate Rust crate used as a path dependency for Collider/signaling development.
- The `rtc/` directory is the local `rtc` repo used during the migration.
- The new architecture is intentionally landing in phases; the absence of full SFU
  behavior in `src/` is expected at this stage.

## Open Source License

This project uses dual licensing under MIT or Apache-2.0.

## Contributing

Contributors and pull requests are welcome.
