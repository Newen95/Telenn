# Telenn

A modular, cross-platform DAW (Windows/Linux) built with Tauri, Rust and Vue 3.
The audio engine runs on a dedicated real-time thread; the whole project state is
plain, diffable text so it can be driven from the UI or by editing files.

## Stack

- **Frontend** — Tauri v2 + Vue 3 + TypeScript (custom canvas timeline/piano roll,
  node graph editor).
- **Backend** — Rust: real-time audio via `cpal`, lock-free ring buffers via
  `rtrb`, WAV export via `hound`, MIDI via `midir`. DSP is hand-written.

See `ARCHITECTURE.md` for the engine and project format, and `UX_SPEC.md` for the
UI. `MVP_PLAN.md` tracks the milestone plan.

## Setup

A fresh clone needs the Rust toolchain, the Tauri system dependencies and the
npm packages. One script does it all (safe to re-run):

```sh
./scripts/setup.sh           # Linux / macOS
```
```powershell
./scripts/setup.ps1          # Windows
```

The Rust toolchain is pinned in `rust-toolchain.toml`, so `rustup` installs the
exact same version (and clippy/rustfmt) on every machine — lint results are
identical regardless of what is otherwise installed. Bump that file deliberately
to upgrade Rust.

On Windows, Tauri also needs the MSVC C++ build tools and the WebView2 runtime;
`setup.ps1` checks for them and points to the installers.

## Development

```sh
npm run tauri dev    # run the app in development
./scripts/check.sh   # local CI: fmt + clippy + tests + frontend build
```

Engine tests live in `src-tauri/` (`cargo test`).

## License

[MIT](LICENSE)
