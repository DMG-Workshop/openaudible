# AudioShelf

A clean-room audiobook library manager and M4B converter: scan folders, search and
browse books, convert to chapter-preserving M4B with live progress, and cancel jobs.

Built with Tauri 2 (Rust) and React. The pipeline lives in `crates/audioshelf-core`
with no UI dependencies, so it is tested headless against the real FFmpeg.

> **Scope.** AudioShelf imports DRM-free `.m4b`, `.m4a` and `.mp3` files. It does not
> decrypt Audible AAX/AAXC files.

## Layout

```
crates/audioshelf-core/   database, scanner, ffprobe/ffmpeg wrappers, job queue (no UI)
src-tauri/                Tauri shell: commands and events only
src/                      React UI (library grid, search, details, job queue)
migrations/               SQL schema (versioned via PRAGMA user_version)
scripts/                  FFmpeg staging with checksum verification
```

## Develop

Prerequisites: Rust (stable), Node 22+, FFmpeg on PATH for tests.
Linux also needs `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf`.

```bash
npm install
node scripts/fetch-ffmpeg.mjs --local   # dev only: copies your ffmpeg into src-tauri/binaries
npm run tauri dev
```

## Test

```bash
cargo test --workspace                    # core unit + end-to-end tests (needs ffmpeg/ffprobe)
cargo clippy --workspace --all-targets -- -D warnings
npm run typecheck && npx vitest run && npm run build
```

The end-to-end tests generate their own sine-tone fixtures, so no audio is committed.

## Release builds

1. Pin an LGPL FFmpeg build per target in `scripts/ffmpeg-manifest.json` (URL and SHA-256).
   The fetch script refuses to run while any checksum is empty.
2. Build the release binary with `node scripts/fetch-ffmpeg.mjs --target <triple>` followed
   by `npm run tauri build`.
3. Tag `vX.Y.Z` to trigger `.github/workflows/release.yml`, which builds macOS (arm64 and
   x86_64), Linux and Windows bundles into a draft release.

### FFmpeg licensing

Ship an **LGPL** build, configured without `--enable-gpl` or `--enable-nonfree`. Don't ship
`libfdk_aac`; the native `aac` encoder is used. Include FFmpeg's license text and a
source-offer notice in the app's About dialog.

### Signing

- **macOS:** Developer ID signing and notarization. Sign the bundled `ffmpeg` and `ffprobe`
  with the same identity, or notarization fails.
- **Windows:** Authenticode-sign the app and the bundled `ffmpeg.exe`.
- **Linux:** publish SHA-256 sums alongside the bundles.

## Not yet decided

- **License.** No license file is included; choose one before publishing.
- **Metadata editing, cover fetching and device sync** are not implemented yet.
