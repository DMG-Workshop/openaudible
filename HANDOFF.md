# AudioShelf: handoff

Snapshot as of 2026-11-01. Repo: `DMG-Workshop/openaudible`.

## Current state

- `main` is at `4b65877`. It contains PR #1 (scaffold) and PR #2 (Windows test build CI).
- `claude/friendly-tesla-zx4u9l` is at `73062d3`, identical to the merged PR #2 head.
- No open PRs. The working tree was clean when this was written.
- CI on `main`'s PR #2 head was green: `test` (Ubuntu) and `windows-test-build`.

## What it is

A clean-room audiobook library manager and M4B converter. It handles only DRM-free files: `.m4b`, `.m4a`, `.mp3`.

It does not and will not handle Audible AAX/AAXC files. Decryption is out of scope, and the project should not add a downloader that logs into audible.com.

## Layout

```
crates/audioshelf-core/   Tauri-free core
  src/db.rs               SQLite (WAL), migrations via PRAGMA user_version, with_db helper
  src/library.rs          FTS5 search, book detail
  src/scan.rs             folder walk, diff by (size, mtime), dirty folders, persist
  src/jobs.rs             job queue (Semaphore), cancel, EventSink trait, sanitize, unique_path
  src/media/probe.rs      ffprobe JSON parsing, tag lookup
  src/media/convert.rs    ffmpeg run (-progress pipe:1), ffmetadata writer, .part rename
  tests/pipeline.rs       end-to-end tests against real ffmpeg/ffprobe
migrations/0001_init.sql  books, book_files, chapters, jobs, books_fts + triggers
src-tauri/                Tauri 2 shell: commands + TauriSink; no business logic
  tauri.conf.json, capabilities/default.json, icons/
src/                      React UI
  App.tsx, main.tsx, styles.css
  components/Toolbar.tsx, LibraryView.tsx, MetadataSidebar.tsx, JobQueue.tsx
  lib/api.ts              invoke wrappers, formatDuration
  state/jobs.ts           zustand store, useJobEvents
scripts/
  fetch-ffmpeg.mjs        stages sidecars; fails closed unless --local
  ffmpeg-manifest.json    per-target url + sha256: ALL EMPTY
.github/workflows/
  ci.yml                  test (Ubuntu) + windows-test-build (unsigned, 7-day artifact)
  release.yml             tag v* → tauri-action, draft release, 4 targets
```

Key design decisions:
- One folder is one book. Multi-part MP3s in a folder merge; chapters are offset by file duration.
- Missing files are flagged, never deleted. A book is missing only when none of its files are present.
- Conversion writes `<output>.part`, then renames on success. Cancel deletes the partial.
- Core emits progress through the `EventSink` trait. Tauri forwards it on `job://progress`.
- Dependency direction: `src-tauri` → `audioshelf-core`. The core must stay Tauri-free.

## Build, test, run

```bash
npm install
node scripts/fetch-ffmpeg.mjs --local      # dev only: copies this machine's ffmpeg into src-tauri/binaries
npm run tauri dev

cargo test --workspace                     # 12 tests; needs ffmpeg and ffprobe on PATH
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm run typecheck && npx vitest run && npm run build
```

Last verified results: 12 Rust tests pass. Clippy with `-D warnings` is clean. fmt is clean. 5 Vitest tests pass. Release binary runs under Xvfb with no panics.

Local-only files to keep out of git: `src-tauri/binaries/` (gitignored), `target/`, `node_modules/`, `dist/`. The `binaries/` copies are the Ubuntu FFmpeg and must never ship.

## Windows

- `windows-test-build` (CI, `windows-latest`) builds NSIS and MSI and uploads `audioshelf-windows-test-unsigned`.
- The bundled FFmpeg there is a GPL Chocolatey build, for testing only.
- The first run failed on the upload path, not the build. The Cargo workspace puts output in the root `target/`. Fixed in `73062d3`.
- Not verified on a real Windows machine. The Windows concat-demuxer path handling (backslashes) is untested.

## Open items (priority order)

1. **License.** None chosen. Blocks any public release. Pick one and add `LICENSE`.
2. **FFmpeg pins.** `scripts/ffmpeg-manifest.json` has empty url and sha256 for all four targets. Release builds fail closed until filled. Use an LGPL build, no `--enable-gpl` or `--enable-nonfree`, and don't ship `libfdk_aac`.
3. **Signing.** macOS: Developer ID and notarization, and the bundled ffmpeg must be signed with the same identity. Windows: Authenticode on the app and `ffmpeg.exe`.
4. **Untested UI paths.** The native folder picker (`plugin-dialog` `open`) has only been driven manually, never by a test. There is no GUI test.
5. **Cover art.** `cover_path` is always null. `Probe.has_cover` is parsed but never extracted or stored.
6. **Metadata editing** is not implemented. Narrator falls back to the composer tag, which is a heuristic.
7. **Cleanup.** `package.json` includes `@testing-library/react` and `jsdom`, but no test uses them. Remove them or add the tests.
8. **Windows-specific code** is not exercised locally.
9. **Not built:** device sync, per-book settings, an updater.

## Known gotchas

- `windows_subsystem` is set in `src-tauri/src/main.rs` for release builds, so no console appears on Windows.
- The Tauri `protocol-asset` feature is required because `tauri.conf.json` enables the asset protocol. Without it, the build script fails.
- `tauri-build` needs the sidecar files to exist. `binaries/ffmpeg-<triple>` must be present before `cargo build` for the shell.
- On the Linux sandbox, `audioDir()` can fail. The app falls back to `homeDir()`.
- Tests: the `pipeline` tests call `Runtime::block_on` nowhere. They must stay `async` under `#[tokio::test]`. A nested `block_on` panics.
- `books.missing` is derived at the end of each scan. Don't set it per file.

## Process notes

- The repo had no commits when the scaffold landed. `main` was created as an empty root commit (`a4b1ea9`), with the scaffold on top.
- The designated branch was restarted from `origin/main` after PR #1 merged. A later force-update to it was blocked by the auto-mode classifier and then approved by the user.
- Commit attribution: the session link and the Claude Code footer are included. The model name is omitted, per the system prompt.
- PR bodies end with the Claude Code footer and the generated-by line, per the repo instructions.

## Requests declined

- A clean-room OpenAudible or Libation clone that logs into audible.com and downloads or decrypts owned books. Declined: it is the same DRM-circumvention pipeline. Alternatives offered were a catalog built from user-supplied data and a design doc for legitimate DRM-free workflows.

## Where to start next

1. Choose a license and add `LICENSE`.
2. Pin the FFmpeg manifest and confirm a release build on each OS.
3. Add a cover extractor, using the attached-picture stream, and store `cover_path`.
4. Add a metadata editor backed by the `books` row, with a `book_files` override path.
5. Remove or use the unused test deps.
