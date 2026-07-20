# Operations

## Source installation

Build the Rust binary first (`cargo build --release`), then run
`./scripts/vwactl setup`. Setup creates a Python `.venv` under the data directory
for CosyVoice and FunClip vendor dependencies only (the HTTP/MCP service itself
is Rust). Rust 1.75+, Python 3.10, uv, FFmpeg, SoX, and Git LFS are required.
Never run setup or a model download merely to inspect the project.

```bash
git clone --recurse-submodules <repo-url>
cd video-work-api
cargo build --release
./scripts/vwactl setup
./scripts/vwactl init
./scripts/vwactl model download
# optional: export VWA_MCP_TOKEN=...
./scripts/vwactl serve
```

In a packaged install, run administrative commands through `sudo vwactl`. The
wrapper re-executes setup, init, model, import, status, and paths as the dedicated
service account. Do not bypass that wrapper or create root-owned files in
`/var/lib/video-work-api`.

Run `vwactl init` once. It creates private data directories, the SQLite
database, and a mode-0600 one-time token. Enter that token in the web setup page
and create a password of at least 12 characters. Successful setup removes the
token. Do not paste it into logs or chat.

Run `vwactl model download` only with explicit approval for the multi-gigabyte
download. It pins both the repository and revision and requests only the files
listed in `scripts/download_model.py`.

Start a foreground development service with `vwactl serve`. Defaults are
`0.0.0.0:7860` and `~/.local/share/video-work-api`. Inspect effective
locations with `vwactl paths` and configuration readiness with `vwactl status`.

Environment variables use the `VWA_*` prefix. See `config.env.example`.

## Reference audio

Use clean speech without music, effects, overlap, clipping, or long silence.
Accept 5–30 seconds; recommend 8–15 seconds. Supply the exact spoken transcript.
Each upload is an independently selectable style.

For batch import, create `<root>/<speaker>/<style>.<ext>` plus sibling
`<style>.txt`, then run `vwactl import <root> --confirm-rights`. Supported
extensions are WAV, MP3, M4A, FLAC, OGG, and WebM. Symlinks, duplicate style stems,
missing transcripts, and unsupported files are rejected.

Prefer the web recorder when the operator is physically recording a new
reference. Browser microphone access requires HTTPS or localhost. For LAN
recording, configure both `VWA_SSL_CERTFILE` and `VWA_SSL_KEYFILE` as regular,
non-symlink certificate/key files, or use an HTTPS reverse proxy. Never invent,
generate, or expose private keys.

## Subtitles (FunClip)

Place videos under `VWA_VIDEO_INPUT_DIR` (default `$VWA_DATA_DIR/videos`).
Call `POST /api/videos/subtitles` or MCP tool `extract_video_subtitles` with a
path relative to that directory. FunClip stage-1 produces time-coded SRT segments.

Default FunClip root is `vendor/FunClip` when present; override with
`VWA_FUNCLIP_ROOT`. First ASR run may download FunASR models.

## MCP for AI agents

Set `VWA_MCP_TOKEN` to a long random secret. Clients POST JSON-RPC to
`http://HOST:PORT/mcp` with `Authorization: Bearer <token>`. Put agent-readable
reference audio under `VWA_REFERENCE_INPUT_DIR` for `add_voice_profile`.

## systemd

The service is `video-work-api.service`. Use `systemctl status`, then
explicit `sudo systemctl start`, `stop`, or `restart`. Installation must not run
`enable`. Read logs with `journalctl -u video-work-api.service` while
avoiding output that contains private text or paths.

Installed paths are `/usr/lib/video-work-api`,
`/etc/video-work-api/config.env`, and
`/var/lib/video-work-api`. The service account must own the data path.

## Troubleshooting

- Run `scripts/health-check.sh` and `vwactl status` first.
- If conversion fails, verify FFmpeg can decode the source and that the result
  would be 5–30 seconds.
- If model loading fails, verify the pinned source submodule and model directory;
  do not download a different model as a shortcut.
- If LAN access fails, confirm the listen address and port before inspecting the
  firewall. Never broaden firewall access without authorization.
- CPU inference is slow. CUDA uses fp16; ONNX frontend sessions remain on CPU.
- macOS Apple Silicon is experimental, foreground-only, and has no systemd path.
- Subtitle extraction requires FunClip with `funclip/videoclipper.py`.
- Videos must stay under `VWA_VIDEO_INPUT_DIR`; reference audio for MCP under
  `VWA_REFERENCE_INPUT_DIR`.
- MCP without a configured token always returns 401.
