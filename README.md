# MultiMouse

Share one mouse and keyboard across Mac, Windows, and Linux. Push your cursor to the edge of the screen and it crosses over to the next machine — keyboard follows. A modern, end-to-end encrypted alternative to Synergy and Barrier, built from scratch with Tauri.

[![Build & Release](https://github.com/coz-whynot/MultiMouse/actions/workflows/build.yml/badge.svg)](https://github.com/coz-whynot/MultiMouse/actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)](#download)
[![Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB)](https://tauri.app)

## Features

- **Seamless cursor handoff** — push to a configurable screen edge (left / right / top / bottom) to control another machine.
- **Keyboard follows cursor** — typing goes to whichever machine the cursor is on.
- **LAN auto-discovery** — nearby devices appear automatically via mDNS.
- **Internet relay** — connect over the internet using a 6-character room code (no port forwarding).
- **End-to-end encryption** — X25519 key exchange + ChaCha20-Poly1305 (MitM-resistant pairing).
- **Clipboard sync** — copy on one machine, paste on another.
- **File drop** — drag files between paired machines on the same LAN.
- **Phone as trackpad** — scan a QR code, use your phone's browser as a trackpad.
- **Auto-reconnect** — paired devices reconnect automatically using persistent session keys.
- **Auto-update** — signed updates delivered via GitHub Releases.
- **Lightweight** — ~10 MB installer, lives in the menu bar / system tray.

## Download

Grab the latest installer from the [Releases page](https://github.com/coz-whynot/MultiMouse/releases/latest):

| Platform | File |
|---|---|
| macOS (Apple Silicon) | `MultiMouse_*_aarch64.dmg` |
| macOS (Intel)         | `MultiMouse_*_x64.dmg` |
| Windows 10 / 11       | `MultiMouse_*_x64-setup.exe` |
| Linux (x86_64)        | `MultiMouse_*_amd64.AppImage` |

### First-run setup

- **macOS** — Open the DMG, drag MultiMouse to Applications. On first launch, grant access in **System Settings → Privacy & Security → Accessibility** and **Input Monitoring**.
- **Windows** — Run the installer. SmartScreen may warn on first run; click **More info → Run anyway**.
- **Linux** — `chmod +x MultiMouse_*.AppImage && ./MultiMouse_*.AppImage`. Requires `libwebkit2gtk-4.1`, `libayatana-appindicator3`, and `libxdo`.

## How it works

1. Launch MultiMouse on two machines on the same network.
2. Each device appears in the other's device list (radar view).
3. Click **Pair** → enter the 6-digit PIN shown on the other side → click **Accept**.
4. Configure the edge direction in **Settings** (which screen sits on which side).
5. Push your cursor to that edge. It crosses over. Keyboard follows.

For connections across the internet, open **Internet** → create a room → share the 6-character code → the other side joins via the relay server.

## Architecture

```
┌─────────────────────────────┐       ┌─────────────────────────────┐
│     MultiMouse (host)       │       │    MultiMouse (client)      │
│                             │       │                             │
│  rdev ─ capture input       │       │     inject input ─ enigo    │
│     │                       │       │          ▲                  │
│     ▼                       │       │          │                  │
│  encrypt (ChaCha20-Poly)    │◄─────►│  decrypt (ChaCha20-Poly)    │
│     │                       │  TCP  │                             │
│  mDNS / relay-server        │       │                             │
└─────────────────────────────┘       └─────────────────────────────┘
```

- **Frontend** — React 19 + TypeScript + Tailwind 4 + Framer Motion, in a frameless 420×680 Tauri window.
- **Backend** — Rust (Tauri 2), `tokio` runtime, `rdev` for capture (with `unstable_grab`), `enigo` for injection, `mdns-sd` for discovery.
- **Crypto** — `x25519-dalek` + `chacha20poly1305` + `hkdf` (SHA-256). Pairing uses short authentication strings to resist MitM.
- **Transport** — TCP over LAN, or TCP splice via `relay-server` for internet sessions.

### Key files

| Path | Purpose |
|---|---|
| [src-tauri/src/lib.rs](src-tauri/src/lib.rs) | Tauri setup, tray, plugin registration |
| [src-tauri/src/state.rs](src-tauri/src/state.rs) | App state (peers, settings, transfers) |
| [src-tauri/src/commands.rs](src-tauri/src/commands.rs) | All `tauri::command` handlers |
| [src-tauri/src/network/](src-tauri/src/network/) | server / client / relay / protocol / transfer |
| [src-tauri/src/input/](src-tauri/src/input/) | capture (rdev) + inject (enigo) |
| [src-tauri/src/crypto/](src-tauri/src/crypto/) | X25519 + ChaCha20-Poly1305 |
| [src-tauri/src/screen/layout.rs](src-tauri/src/screen/layout.rs) | Edge detection |
| [src/App.tsx](src/App.tsx) | Event wiring, error banner, pairing modal |
| [src/pages/](src/pages/) | Home, Layout, Settings |
| [relay-server/](relay-server/) | Standalone TCP relay (room-code based) |
| [.github/workflows/build.yml](.github/workflows/build.yml) | CI: matrix build + signed releases |

## Development

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable)
- [Node.js](https://nodejs.org) 20+
- Tauri [platform prerequisites](https://v2.tauri.app/start/prerequisites/)

**Linux** also needs:

```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  libxdo-dev libxtst-dev libx11-dev libxcb1-dev \
  libxrandr-dev libxi-dev pkg-config
```

### Run locally

```bash
git clone https://github.com/coz-whynot/MultiMouse.git
cd MultiMouse
npm install
npm run tauri dev
```

### Build a release bundle

```bash
npm run tauri build
# Artifacts in src-tauri/target/release/bundle/
```

### Relay server

```bash
cargo run --release --manifest-path relay-server/Cargo.toml
# Listens on 0.0.0.0:57173 (override with RELAY_PORT env var)
```

## Cutting a release

Releases are fully automated via GitHub Actions ([.github/workflows/build.yml](.github/workflows/build.yml)):

```bash
# bump version in package.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml
git tag v0.2.0
git push --tags
```

The workflow builds signed installers for macOS (arm64 + x64), Windows, and Linux, publishes a draft GitHub release, and uploads `latest.json` so existing installs auto-update.

## Security

- All inter-device traffic is encrypted with ChaCha20-Poly1305 using a session key derived from X25519 ECDH.
- Pairing displays a short authentication string on both sides to detect MitM; always verify it matches before accepting.
- Session keys are persisted locally in the app data directory (`config.json`); delete the entry in **Settings → Paired devices** to revoke.

Found a vulnerability? Please open a private security advisory on GitHub rather than a public issue.

## Roadmap

- Screen layout editor (drag monitors to arrange)
- Multi-monitor support for relay sessions
- Configurable relay-server room timeout
- mDNS deregistration on clean disconnect

## Contributing

Pull requests welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for build setup, coding conventions, and the PR checklist.

## License

[MIT](LICENSE) © MultiMouse contributors.

Built on the shoulders of [Tauri](https://tauri.app), [rdev](https://github.com/Narsil/rdev), and [enigo](https://github.com/enigo-rs/enigo).
