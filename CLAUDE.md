# MultiMouse

Cross-platform mouse/keyboard sharing app (Synergy/Barrier-style). Tauri 2.0 + Rust backend + React/TS/Tailwind frontend. Repo: https://github.com/coz-whynot/MultiMouse

## Editing rules (read before every edit)

Follow this checklist for every change, no exceptions:

1. **Re-read this file.** Rules change; don't rely on memory of a prior session.
2. **Deep check before editing.** Don't skim. Understand the data flow, the state it touches, and what else depends on it.
3. **End-to-end trace.** Follow the change from UI event → `commands.rs` → state/network → peer → injection (or the reverse). A fix that looks local often isn't.
4. **Read the whole file or function first.** Never edit a function without reading it top-to-bottom. Never edit a module without scanning its other functions for shared state, invariants, or sibling patterns you'd be breaking.
5. **Explain the change in plain English before you make it.** State: *what* will change, *why*, whether it's a good or bad change, and the tradeoffs. Wait for acknowledgment on non-trivial edits.
6. **No patch/band-aid fixes.** Fix the root cause. A `.unwrap_or_default()` that silences a panic, a `try/catch` that swallows an error, a special-case branch for one caller — these are red flags. Only patch if the root cause is genuinely out of scope, and say so explicitly when you do.

### Additional rules that apply to this project specifically

7. **Check all three targets mentally** (macOS, Windows, Linux) for any change in [src-tauri/src/input/](src-tauri/src/input/) or anything involving `enigo`, `rdev`, or OS APIs. If a variant is `#[cfg]`-gated, the match arm must be too.
8. **Verify both sides of the wire.** Any change to [network/protocol.rs](src-tauri/src/network/protocol.rs), [server.rs](src-tauri/src/network/server.rs), or [client.rs](src-tauri/src/network/client.rs) must be checked against its counterpart — the peer runs the *same* binary but as the opposite role. Asymmetric changes cause silent hangs.
9. **Protocol changes are breaking.** Adding/removing/renaming a `Message` variant breaks older installs out there. Add new variants additively; don't reorder or repurpose existing ones.
10. **Respect the storage boundary.** All persisted config goes through [storage.rs](src-tauri/src/storage.rs). Don't read/write `config.json` from other modules.
11. **Don't touch the hot paths casually.** The rdev capture loop, the 120 Hz `thread_local` throttle, and the enigo injection path are performance- and deadlock-sensitive. No `.unwrap()`, no blocking I/O, no lock acquisition inside them. Any change here needs a specific justification.
12. **Async vs sync discipline.** Clipboard is sync-only (spawns `std::thread`). rdev's grab loop is sync and platform-threaded. Don't move either onto the Tokio runtime.
13. **Verify before reporting done.** After Rust edits: `cargo check` (or `cargo build`) must pass for the target you can compile locally. After frontend edits: `npm run build` (runs `tsc` then `vite build`) must pass. State explicitly if you could not test the other platforms.
14. **Preserve cross-platform build.** If you add a dependency, confirm it compiles on all three OSes or gate it with `#[cfg]`. Don't introduce Unix-only crates without guards.
15. **UI changes need visual confirmation.** For anything in [src/](src/), run `npm run tauri dev` and exercise the feature before marking it done. If you can't, say so.
16. **Summon specialist agents deliberately, not reflexively.** Use the `Agent` tool with the right `subagent_type` when the task genuinely benefits — not for every small lookup.
    - **`Plan`** — before any non-trivial change (new feature, refactor touching >2 files, anything cross-cutting server/client, protocol changes). Get a step-by-step plan, then execute it. Don't plan in your head for complex work.
    - **`Explore`** — when you need to understand unfamiliar code across several files, or find where a concept lives in the codebase. Cheaper than reading 10 files yourself and keeps the main context clean.
    - **`general-purpose`** — open-ended research, multi-step searches where you're not confident the first query will hit.
    - **`claude-code-guide`** — Claude Code / Agent SDK / Anthropic API questions only.
    - **Parallelize independent work.** If two research questions don't depend on each other, launch both agents in one message so they run concurrently.
    - **Brief them like a colleague who just walked in.** Self-contained prompt: goal, context, what's been tried, what to report. Terse prompts produce shallow work.
    - **Never delegate understanding.** Don't write "based on your findings, fix the bug." Synthesize the agent's report yourself, then act. Verify changes they claim to have made by reading the actual diff.
    - **Plan the agent usage up front.** For a multi-step task, decide at the start which steps warrant an agent and which you'll do directly. Don't spawn agents ad-hoc mid-task.

## Stack

- **Backend:** Rust (Tauri 2.0), `rdev` (capture, uses `unstable_grab`), `enigo 0.2` (injection), mDNS discovery, TCP transport
- **Frontend:** React 19 + TypeScript + Tailwind 4 + Zustand + framer-motion
- **Relay:** Standalone Rust TCP binary in [relay-server/](relay-server/)
- **CI:** [.github/workflows/build.yml](.github/workflows/build.yml) — macOS arm64/x64, Windows x64, Linux x64

## Layout

- [src-tauri/src/lib.rs](src-tauri/src/lib.rs) — Tauri setup, tray, plugin registration
- [src-tauri/src/state.rs](src-tauri/src/state.rs) — `AppState` (peers, settings, transfers, pending_pairing)
- [src-tauri/src/storage.rs](src-tauri/src/storage.rs) — `config.json` persistence
- [src-tauri/src/commands.rs](src-tauri/src/commands.rs) — all `#[tauri::command]` handlers
- [src-tauri/src/network/](src-tauri/src/network/) — `server.rs`, `client.rs`, `relay.rs`, `transfer.rs`, `protocol.rs`
- [src-tauri/src/input/](src-tauri/src/input/) — `capture.rs` (rdev), `inject.rs` (enigo)
- [src-tauri/src/screen/layout.rs](src-tauri/src/screen/layout.rs) — edge detection
- [src/App.tsx](src/App.tsx), [src/pages/](src/pages/), [src/components/](src/components/)

## Build / run

- Dev: `npm run tauri dev`
- Frontend only: `npm run dev`
- Release build: `npm run tauri build`
- Release cut: `git tag vX.Y.Z && git push --tags` — CI signs, creates draft release, generates `latest.json` for auto-updater

## Project-specific rules

### enigo key/button variants are platform-gated

Several `enigo::Key` / `enigo::Button` variants are `#[cfg(...)]`-gated in enigo 0.2 and **do not exist** on macOS. Match arms for these must be wrapped with the matching `#[cfg]` or macOS won't compile ("no variant found"):

- `Key::Insert`, `Key::Print`, `Key::Pause`, `Key::Numlock` (lowercase `l`) — `#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]`
- `Key::ScrollLock` — `#[cfg(all(unix, not(target_os = "macos")))]` (Linux only)
- `Button::Back`, `Button::Forward` — same cfg as `Insert`

On macOS these fall through to the `_` arm (silent skip) — that's correct; the OS handles them differently.

### Tauri artifact path gotcha

When `--target <triple>` is passed, Tauri outputs to `target/<triple>/release/bundle/`, **not** `target/release/bundle/`. CI upload paths must use `src-tauri/target/${{ matrix.target }}/release/bundle/...`.

### Auto-updater signing

- Private key lives in GitHub secret `TAURI_SIGNING_PRIVATE_KEY` (no password).
- Public key is embedded in [src-tauri/tauri.conf.json](src-tauri/tauri.conf.json) under `plugins.updater.pubkey`.
- Updater endpoint: `https://github.com/coz-whynot/MultiMouse/releases/latest/download/latest.json` (tauri-action uploads this automatically on tag release).
- **If the private key is lost, auto-updates break for all existing installs.** Regenerate with `npx tauri signer generate -p "" -w <path>`, then update both `pubkey` in `tauri.conf.json` and the GitHub secret.

### Apple signing

Not configured — no `APPLE_CERTIFICATE` secret. Manual `workflow_dispatch` builds skip Apple signing env vars entirely (empty values crashed `security import`). Release builds pass empty Apple vars; tauri-action degrades to an unsigned DMG. To enable notarization, add `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`.

### Pairing / session model

- First connect: initiator sends `PinRequest` → receiver sees Accept/Reject modal → on Accept, server generates a 32-char hex session key, stores it, returns to client.
- Subsequent connects use `SessionAuth` (session key), not PIN. Keys persist in `config.json`.

### Known limitations

- File transfer: LAN only (blocked over relay with a clear error — don't "fix" this by routing through relay).
- Relay uses primary monitor only (multi-monitor relay not implemented).
- mDNS deregistration on disconnect is not implemented.
- Relay room timeout is hardcoded.

## Conventions

- Mouse move is throttled to ~120 Hz via a `thread_local` `Cell` in the capture loop — don't remove without a replacement rate limiter.
- Clipboard sync spawns a `std::thread` on focus change (non-blocking) — keep it off the async runtime; the clipboard crate is sync-only.
- All persisted state goes through [storage.rs](src-tauri/src/storage.rs); don't write `config.json` directly from other modules.
