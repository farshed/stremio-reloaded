# Stremio Shell (Tauri)

Desktop shell for Stremio built with Tauri. It loads the Stremio web UI, runs the streaming server (`server.js` on Node) as a sidecar, and plays video with libmpv.

## Requirements

- Rust and Bun
- libmpv (macOS: `brew install mpv`). Set `MPV_LIB_DIR` if it is not found through `pkg-config`.
- ffmpeg and ffprobe on your `PATH` for development, or `FFMPEG_DIR` pointing to static builds.

## Setup

```sh
bun install
bun run sidecars   # downloads Node and server.js, copies ffmpeg/ffprobe
```

## Run and build

```sh
bun run dev
bun run build
```

## Release

1. Bump `version` in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`.
2. Add a `## <version>` section to `.github/release/CHANGELOG.md`.
3. Push a tag: `git tag v<version> && git push origin v<version>`.

The `release` workflow builds macOS (arm64, Intel), Linux (x64, arm64) and Windows (x64), then publishes a GitHub release.

macOS signing is optional. Without secrets the app is ad-hoc signed. Set `MACOS_CERTIFICATE` (base64 .p12), `MACOS_CERTIFICATE_PASSWORD` and `MACOS_SIGNING_IDENTITY` to sign it, and add `APPLE_NOTARIZATION_KEY` (.p8 contents), `APPLE_NOTARIZATION_KEY_ID` and `APPLE_NOTARIZATION_ISSUER_ID` to notarize it.

## Options

- `--development`: load the web UI from `http://127.0.0.1:11470` and do not start the streaming server
- `--streaming-server`: with `--development`, start the streaming server anyway
- `--staging`: load the web UI from `https://staging.strem.io`
- `--webui-url=<url>`: load the web UI from a custom URL

## Status

Video output is implemented on macOS only. Windows and Linux currently play audio only.
