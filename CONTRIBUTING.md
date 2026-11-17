# Contributing to Go Player

Thanks for your interest! Go Player is a media player whose core
differentiator is its subtitle intelligence system — contributions that
respect playback performance, offline capability, and privacy are the
most valuable.

Please read [`AGENTS.md`](AGENTS.md) before writing code. It defines the
architecture boundaries, engineering workflow, and definition of done.
AI-assisted contributions are welcome — the repo ships an agent skill at
[`.agents/skills/go-player-engineering/SKILL.md`](.agents/skills/go-player-engineering/SKILL.md).

## Prerequisites

| Tool     | Version / notes                                        |
| -------- | ------------------------------------------------------ |
| Rust     | Pinned in `rust-toolchain.toml` (MSVC on Windows)      |
| Node.js  | Pinned in `mise.toml`                                  |
| pnpm     | Pinned in `mise.toml` — run `pnpm install` at the root |
| Windows  | VS 2022 Build Tools, *Desktop development with C++*    |

One-command setup (idempotent): `./scripts/setup.ps1` (Windows) or
`./scripts/setup.sh` (Linux/macOS). `mise install` pins Node/pnpm.

## Development workflow

```bash
pnpm install
pnpm dev          # tauri dev — frontend + Rust watch mode
pnpm build        # frontend production build
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

Repo layout:

- `apps/app/` — Tauri host (`src/` Solid frontend, `src-tauri/` Rust)
- `crates/` — Rust workspace crates (`go-media`, `go-subtitles`, …)
- `packages/contracts/` — TypeScript types mirroring Rust IPC shapes
- `docs/decisions/` — ADRs for significant decisions

## Ground rules

- Playback responsiveness beats everything — never block the UI or
  media threads with AI, disk, or network work.
- Keep engine/AI implementation types out of public domain interfaces.
- No cloud uploads, telemetry, or analytics without explicit approval.
- Validate all external input (paths, subtitles, model metadata).
- Small, focused changes. Write tests for new behavior.
- Update docs (`AGENTS.md`, ADRs, README) when behavior or decisions change.

## Pull requests

- Open an issue first for non-trivial features so the approach can be
  discussed.
- Keep PRs focused; one concern per PR.
- Ensure `cargo fmt`, `clippy`, `cargo test`, and `pnpm build` pass.
- Describe what changed, how you verified it, and any limitations.

## Reporting bugs

Use the bug report template. Include OS, app version, media file details
(container/codec), and steps to reproduce. For playback issues, note
whether the file plays in other players (mpv, VLC) — it helps isolate
demux/decode/render problems.

## Licensing

Licensing is not yet finalized — see `AGENTS.md` §13. By contributing
you agree your contributions may be relicensed as needed when a license
is chosen. Do not commit code or assets with incompatible licenses.
