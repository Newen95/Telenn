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

## Development

```sh
npm install          # install frontend dependencies
npm run tauri dev    # run the app in development
./check.sh           # local CI: fmt + clippy + tests + frontend build
```

Engine tests live in `src-tauri/` (`cargo test`).
