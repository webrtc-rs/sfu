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

The current codebase has completed the workspace split for the new design:

- `crates/sfu-engine` — the new sans-IO SFU core boundary
- `crates/sfu-driver` — the future socket/runtime driver layer
- `apprtc` — the Rust AppRTC signaling / Collider development crate
- `crates/sfu-bin` — the runnable binary entrypoint

The new workspace is the active implementation target, with additional repo
artifacts such as `docs/`, `scripts/`, and `todo_tests/` still present alongside
it during the migration.

## Repository Layout

```text
sfu/
├── Cargo.toml              # workspace root
├── apprtc/                 # apprtc submodule
├── crates/
│   ├── sfu-bin/            # binary scaffold
│   ├── sfu-driver/         # driver scaffold
│   └── sfu-engine/         # sans-IO SFU core scaffold
├── rtc/                    # rtc submodule
├── scripts/                # helper scripts
├── docs/                   # project documentation
└── todo_tests/             # migration/reference tests and examples
```

## Building

### Toolchain

Use a Rust toolchain with Edition 2024 support.

### Build the workspace

```bash
cargo build --workspace
```

### Format the workspace

```bash
cargo fmt --all
```

### Run the current binary scaffold

```bash
cargo run -p sfu-bin
```

## Notes for Contributors

- The active implementation now lives under `crates/`.
- The `apprtc/` directory is where Rust-based Collider/signaling development now lives.
- The `rtc/` directory is the local `rtc` workspace used during the migration.
- The new architecture is intentionally landing in phases; the absence of full SFU
  behavior in `crates/` is expected at this stage.

## Open Source License

This project uses dual licensing under MIT or Apache-2.0.

## Contributing

Contributors and pull requests are welcome.
