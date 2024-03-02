<h1 align="center">
 <a href="https://sfu.rs"><img src="https://raw.githubusercontent.com/webrtc-rs/sfu/master/docs/sfu-rs.jpg" alt="SFU"></a>
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
</p>
<p align="center">
 SFU in Rust with Sans-IO
</p>

#

<details>
<summary><b>Table of Content</b></summary>

- [Building](#building)
    - [Toolchain](#toolchain)
    - [Monorepo Setup](#monorepo-setup)
- [Open Source License](#open-source-license)
- [Contributing](#contributing)
</details>

#

## Building

### Toolchain

SFU.rs currently requires Rust 1.75.0+ to build.

### Monorepo Setup

All [RTC](https://github.com/webrtc-rs/rtc/) protocols are added as [submodules](https://git-scm.com/book/en/v2/Git-Tools-Submodules) of this repository under /rtc/.

```
git clone https://github.com/webrtc-rs/sfu
cd sfu
git submodule update --init --recursive
```

To build sfu crate:

```
cargo build [or clippy or test or fmt]
```

To build chat examples:

```
cargo run --package sfu --example sync_chat
cargo run --package sfu --example async_chat
```

## Open Source License
Dual licensing under both MIT and Apache-2.0 is the currently accepted standard by the Rust language community and has been used for both the compiler and many public libraries since (see https://doc.rust-lang.org/1.6.0/complement-project-faq.html#why-dual-mitasl2-license). In order to match the community standards, SFU.rs is using the dual MIT+Apache-2.0 license.

## Contributing
Contributors or Pull Requests are Welcome!!!
