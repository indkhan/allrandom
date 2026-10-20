# Ownkit

Free, open-source desktop toolbox (Tauri + Rust) for the little file jobs you keep googling for. Everything runs on your computer: no uploads, no accounts, no watermarks.

- **Squeeze**: compress videos to a smaller size (bundled ffmpeg)
- **Trim & GIF**: cut a clip or turn it into a GIF
- **Compress files**: pack files and folders into a .zip
- **Merge**: combine PDFs and images (JPG, PNG, WebP, GIF, BMP) into one PDF, in any order
- **Split**: extract pages or ranges from a PDF
- **Compress PDF**: shrink the images inside a PDF
- **Audio**: convert audio or video to MP3, WAV or M4A
- **Images**: convert and resize images

## Download

Grab the installer for your OS from [Releases](https://github.com/indkhan/allrandom/releases/latest):

| OS | File |
|---|---|
| Windows | `.exe` (setup) or `.msi` |
| macOS (Apple Silicon) | `.dmg`. The app is unsigned, so right-click it and choose Open the first time |
| Linux | `.AppImage` (`chmod +x` it, then run) |

Once installed, the app checks for new versions on launch and updates itself in one click.

## Squeeze presets

| Preset | What it does |
|---|---|
| Target size | Two-pass encode to hit the MB you type |
| High quality | H.264 CRF 20 |
| Balanced | H.265 CRF 26 |
| Smallest | H.265 CRF 32, capped at 720p |
| Lossless | H.264 lossless into .mkv |

Codec (H.264 / H.265) and resolution (1080p / 720p / 480p) can be overridden.

## Build

Needs Rust, Node, and on Windows the VS C++ Build Tools.

```bash
./get-ffmpeg.ps1   # puts ffmpeg into src-tauri/binaries (Windows)
npm install
npm run dev        # run
npm run build      # installer in src-tauri/target/release/bundle
```

On Linux/macOS drop an ffmpeg binary at `src-tauri/binaries/ffmpeg-<target-triple>` instead.

Releases: bump the version in `src-tauri/tauri.conf.json` and `Cargo.toml`, push a `v*` tag, and `.github/workflows/release.yml` builds all three OSes, signs the updates (needs the `TAURI_SIGNING_PRIVATE_KEY` repo secret) and publishes them.

Tests: `cd src-tauri && cargo test` (add `-- --ignored` to run real encodes; needs ffmpeg on PATH).
