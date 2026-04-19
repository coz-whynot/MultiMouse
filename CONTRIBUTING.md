# Contributing to MultiMouse

Thanks for taking the time to contribute. This document covers the basics of getting a dev environment running and the conventions we follow.

## Getting set up

See the [Development section in the README](README.md#development) for prerequisites and run instructions. The short version:

```bash
npm install
npm run tauri dev
```

The frontend lives in [src/](src/) (React + TypeScript + Tailwind). The Rust backend lives in [src-tauri/src/](src-tauri/src/). The standalone relay is in [relay-server/](relay-server/).

## Filing issues

When reporting a bug, please include:

- OS and version (macOS 14.4, Windows 11 23H2, Ubuntu 22.04, …)
- MultiMouse version (shown in **Settings**)
- Steps to reproduce
- Relevant logs — on macOS/Linux run from a terminal to capture stderr; on Windows, run `multimouse.exe` from PowerShell

For feature requests, describe the use case first — what are you trying to do end-to-end? — before jumping to a specific implementation.

## Pull requests

Before opening a PR:

1. **One logical change per PR.** Unrelated cleanups should go in their own PR.
2. **Run the full app.** Type-checks and unit tests don't catch the kind of regressions this project has (input capture, edge detection, encryption). Smoke-test the golden path in `npm run tauri dev` on your platform.
3. **Cross-platform awareness.** `enigo` and `rdev` behave subtly differently on each OS. If you touch [src-tauri/src/input/](src-tauri/src/input/), call that out in the PR description and flag which platforms you tested on.
4. **Don't break pairing.** Changes to [src-tauri/src/crypto/](src-tauri/src/crypto/) or [src-tauri/src/network/protocol.rs](src-tauri/src/network/protocol.rs) affect every paired device; bump the protocol version if the wire format changes.
5. **Keep the installer lean.** Avoid pulling in heavy JS or Rust dependencies if a smaller option exists.

### Commit messages

Short, imperative, lowercase. Examples:

```
add internet relay room codes
fix cursor jitter on edge crossover
bump enigo to 0.2.1
```

If the change needs context, put it in the body — not the title.

### Coding conventions

- **Rust** — `cargo fmt` before committing. Prefer `tracing::{debug,info,warn,error}` over `println!`. Keep `unsafe` out of `input/` and `crypto/` unless there's no alternative.
- **TypeScript** — no implicit `any`; strict mode is on. Use the existing Zustand stores in [src/store/](src/store/) rather than prop-drilling state.
- **No comments explaining what the code does.** Only explain *why* if it's non-obvious.

## Releasing

Maintainers: see [README.md → Cutting a release](README.md#cutting-a-release). The short version is bump the three version fields, tag `vX.Y.Z`, and push the tag — CI handles the rest.

## Code of conduct

Be kind. Assume good faith. Project maintainers may remove comments, commits, or contributors that don't follow this.
