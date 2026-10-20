# AGENTS.md

All: a small Tauri 2 desktop toolbox (Rust backend, plain HTML/JS frontend, no bundler). Free, open source (MIT) and non-commercial: open-source licenses (GPL/AGPL included) are fine for dependencies and optional downloads.

## Layout
- `src/`: frontend, one HTML page per tool (`index.html` is the home screen, `squeeze.html`, `zip.html`, `merge.html`), shared `style.css` and `list.js`, vendored libraries in `src/vendor/`
- `src-tauri/src/main.rs`: every Rust command and its tests
- `src-tauri/tauri.conf.json`: app config, bundle targets, updater
- `.github/workflows/release.yml`: builds and publishes releases on `v*` tags

## Commands
```bash
npm ci                          # install the Tauri CLI
./get-ffmpeg.ps1                # ffmpeg sidecar (Windows); needed before build/test
npm run dev                     # run the app
cd src-tauri && cargo test      # tests; add -- --ignored for real ffmpeg encodes
```

## How to work
- **Simplest thing that works.** Fewest files and fewest lines. No abstractions, config or "for later" code until something actually needs it.
- **Reuse before adding.** Check what's already here (`list.js`, `style.css` tokens, `ffmpeg()`, existing commands and plugins), then the standard library, then a platform feature. Add a new dependency only when none of those can do the job.
- **Match the surrounding code.** Same naming, same comment density. New tools follow the existing page pattern: header with a back link, dropzone, footer with the primary button.
- **One logical change per commit.** Commit as you go with short imperative messages ("Add X", "Fix Y"). Don't bundle unrelated changes.
- **Verify before claiming done.** Run `cargo check`/`cargo test`, and look at UI changes in the app.

## Gotchas
- The webview uses `window.__TAURI__` globals (`withGlobalTauri`). Every plugin needs its permission in `src-tauri/capabilities/default.json`.
- Colors are CSS variables on `:root`, with dark-mode overrides. Never hardcode colors.
- The shell on this machine is often PowerShell: `<` redirection doesn't work there.

## Releasing
1. Bump `version` in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, then commit.
2. `git tag vX.Y.Z && git push origin main vX.Y.Z`. CI builds Windows, macOS and AppImage, and signs the updates.
3. Never move or reuse a published tag; ship a new patch version instead.
4. Signing needs the `TAURI_SIGNING_PRIVATE_KEY` repo secret. Its public key lives in `tauri.conf.json`, and changing it breaks updates for existing installs.
