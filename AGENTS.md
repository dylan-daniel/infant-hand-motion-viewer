# AGENTS.md

## Project

Infant Hand Motion Viewer: a Rust desktop app (wgpu + Dear ImGui) for browsing infant grasping trials, scrubbing frames, and viewing fitted MANO hands in 3D or overlaid on the frame image. Data can be local or streamed from a remote server over SSH.

## Module map

- `src/main.rs`, `src/lib.rs`: app entry point and shared library root
- `src/config.rs`: persisted user settings
- `src/assets.rs`: embedded assets
- `src/data/`: hexport loading, MANO model, mesh sequences, camera projection math
- `src/graphics/`: wgpu renderer, cameras, framebuffers, image textures, hand overlay, WGSL shaders
- `src/ui/`: ImGui panes (explorer, viewport, image view, transport, menus, modals)
- `src/remote/`: SSH remote client
- `src/video/`: remote trial video download, decoding through the `ffmpeg` command-line tool into an in-memory frame store, optional on-disk video cache
- `src/util/`: worker queue, window placement
- `scripts/viewer_daemon.py`: remote-side daemon the Rust remote client talks to; keep both sides in sync
- `tests/`: integration tests

## Runtime requirements

Remote frames come from the trial's mp4, decoded by running the `ffmpeg` and `ffprobe` command-line tools, which must be on `PATH` (or in `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`). The app is not linked against FFmpeg and ships none of it; at startup it shows a native warning box if the tools are missing. Install with `winget install Gyan.FFmpeg`, `brew install ffmpeg` or the distribution's `ffmpeg` package.

## Commands

- `just format`: `cargo fmt`
- `just lint`: clippy with `-D warnings`
- `just check`: fmt check plus lint
- `just test`: `cargo test`

A pre-commit hook (`.githooks/pre-commit`, installed via `just setup`) enforces checks. Do not duplicate it by hand before every commit.

## Commit style

Conventional Commits: `type(scope): short message` or `type: short message`.

- Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `build`, `ci`, `perf`, `style`, `revert`. Use whichever fits.
- Scope is optional and free-form (e.g. `ui`, `graphics`, `remote`).
- Lowercase, imperative, no trailing period, one line.
- No commit body unless a breaking change needs explaining.
- Never add a `Co-Authored-By` trailer or any other attribution line to commits.

### Commit incrementally

One logical change per commit. Never batch a whole task into one commit.
If asked to roll out 3 features, make at least 3 commits, one per feature, more if a feature has separable steps (data, then graphics, then UI, then config). Each commit should build on its own.

### Git rules

- Commit when asked to implement something. Do not push, amend, force-push, or rewrite history unless told to.

## Code style

- Follow `rustfmt.toml` (edition 2024, max width 120). Match surrounding code's naming and idiom.
- Never write verbose comments. At most a one-line `///` on a public item or a non-obvious why. No inline narration, no multi-line comment blocks, no restating the code.
- Add tests in `tests/` for new logic that can run without a GPU.
- Ask before adding a new dependency.
