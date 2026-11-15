# Stremio Reloaded

Debloated Stremio desktop shell built with Tauri. Quicker startup and less jank. macOS and Linux only atm.

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

## Options

- `--development`: load the web UI from `http://127.0.0.1:11470` and do not start the streaming server
- `--streaming-server`: with `--development`, start the streaming server anyway
- `--staging`: load the web UI from `https://staging.strem.io`
- `--webui-url=<url>`: load the web UI from a custom URL

## Status

Video output is implemented on macOS and Linux. Windows currently plays audio only.
