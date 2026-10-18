# Squeeze

Small desktop video compressor (Tauri + Rust, bundled ffmpeg). Windows first; builds on Linux/macOS too.

Drop a video in, pick a preset, compress:

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

Tests: `cd src-tauri && cargo test` (add `-- --ignored` to run real encodes; needs ffmpeg on PATH).
