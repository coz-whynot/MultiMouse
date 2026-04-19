# Changelog

All notable changes to MultiMouse are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned
- Screen layout editor (drag monitors to arrange)
- Multi-monitor support for relay sessions
- Configurable relay-server room timeout
- mDNS deregistration on clean disconnect

## [0.1.0] — 2026-04-19

Initial public release.

### Added
- System tray / menu bar app for macOS, Windows, Linux
- LAN auto-discovery via mDNS
- PIN + Accept/Reject pairing flow with persistent session keys
- Seamless cursor handoff on configurable screen edge (left / right / top / bottom)
- Keyboard-follows-cursor input forwarding via `rdev` (capture) and `enigo` (injection)
- End-to-end encryption: X25519 ECDH + ChaCha20-Poly1305 + HKDF-SHA256
- MitM-resistant pairing with short authentication strings
- Clipboard sync on focus change
- LAN file drop between paired devices
- Internet relay sessions via standalone `relay-server` binary (6-char room codes, TCP splice)
- Phone-as-trackpad (QR code → browser-based trackpad)
- Auto-reconnect using persistent session keys
- Launch on startup (macOS LaunchAgent, Windows registry, Linux `.desktop`)
- Ping/pong keepalive with latency display
- ShareIt-style UI: radar animation, per-device avatars, violet gradient theme
- Onboarding wizard, bottom navigation, Take Control shortcut, error banner
- Auto-update via `tauri-plugin-updater` against GitHub Releases `latest.json`
- GitHub Actions CI: matrix build (macOS arm64/x64, Windows x64, Linux x64) + signed releases

[Unreleased]: https://github.com/coz-whynot/MultiMouse/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/coz-whynot/MultiMouse/releases/tag/v0.1.0
