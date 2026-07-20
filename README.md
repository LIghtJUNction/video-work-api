# Video Work API · 视频工作 API

Self-hosted **toolkit for AI agents**: authorized zero-shot voice cloning
(CosyVoice3) and precise video subtitle extraction (FunClip), exposed as an
authenticated HTTP API and an **HTTP MCP** server.

- Product (zh): **视频工作 API**
- Binary / CLI: `vwactl` · crate: `video-work-api` · default port: `7860`
- Implementation: **Rust** (`src/` layout) with Python helpers only for
  CosyVoice / FunClip model inference
- Voice: Apache-2.0
  [`FunAudioLLM/Fun-CosyVoice3-0.5B-2512`](https://huggingface.co/FunAudioLLM/Fun-CosyVoice3-0.5B-2512)
  revision `29e01c4e8d000f4bcd70751be16fa94bf3d85a18` with vendored
  [`FunAudioLLM/CosyVoice`](https://github.com/FunAudioLLM/CosyVoice)
- Subtitles: [`modelscope/FunClip`](https://github.com/modelscope/FunClip)
  stage-1 ASR (FunASR) with time-coded SRT segments

> Clone and use a voice only with the speaker's explicit permission. Voice
> cloning can facilitate impersonation and fraud. Read [SECURITY.md](SECURITY.md).

## Requirements

- Linux with **Rust 1.75+** (`cargo`), Python 3.10, `uv`, FFmpeg, SoX, and Git LFS
- NVIDIA CUDA is recommended for CosyVoice; CPU inference is possible but slow
- Roughly 10 GB for the CosyVoice3 snapshot plus the Python vendor environment
- FunASR models download on first subtitle extraction

## Install from source

```bash
git clone --recurse-submodules <your-fork-or-repo-url>
cd video-work-api
cargo build --release
./scripts/vwactl setup          # Python venv for CosyVoice/FunClip only
./scripts/vwactl init
./scripts/vwactl model download
export VWA_MCP_TOKEN="$(openssl rand -hex 32)"
./scripts/vwactl serve
```

`vwactl init` prints a one-time setup token. Open `http://127.0.0.1:7860`,
create the administrator password, then manage voice profiles in the UI or via
API/MCP.

Environment variables (prefix `VWA_`): `VWA_DATA_DIR`, `VWA_MODEL_DIR`,
`VWA_COSYVOICE_ROOT`, `VWA_FUNCLIP_ROOT`, `VWA_SETUP_TOKEN_FILE`, `VWA_HOST`,
`VWA_PORT`, `VWA_MCP_TOKEN`, `VWA_VIDEO_INPUT_DIR`, `VWA_REFERENCE_INPUT_DIR`,
`VWA_SUBTITLE_TIMEOUT`, `VWA_PYTHON`, `VWA_PROJECT_ROOT`, optional
`VWA_SSL_CERTFILE` / `VWA_SSL_KEYFILE` (validated; terminate TLS at a reverse
proxy for production HTTPS). Defaults live under `~/.local/share/video-work-api`.
See `config.env.example`.

## Capabilities

### 1. Voice cloning (CosyVoice3)

Upload or import a 5–30 s reference clip with an **exact** transcript and
explicit rights confirmation. Generate speech for any target text (speed
0.75–1.25).

Batch import:

```bash
vwactl import ./voices --confirm-rights
```

### 2. Video subtitles (FunClip)

Place videos under `VWA_VIDEO_INPUT_DIR` (default `$VWA_DATA_DIR/videos`), then:

```bash
curl -sS -X POST "http://127.0.0.1:7860/api/videos/subtitles" \
  -H "Origin: http://127.0.0.1:7860" \
  -H "Content-Type: application/json" \
  -b cookies.txt \
  -d '{"video_path":"clip.mp4"}'
```

Response includes `segments[{index,start,end,text}]` and full `srt`.

### 3. HTTP MCP for agents

```bash
curl -sS -X POST "http://127.0.0.1:7860/mcp" \
  -H "Authorization: Bearer $VWA_MCP_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

Tools: `get_status`, `list_speakers`, `create_speaker`, `delete_speaker`,
`add_voice_profile`, `delete_voice_profile`, `generate_speech`, `get_generation`,
`extract_video_subtitles`.

Example client config (field names vary by host):

```json
{
  "mcpServers": {
    "video-work-api": {
      "url": "http://127.0.0.1:7860/mcp",
      "headers": { "Authorization": "Bearer ${VWA_MCP_TOKEN}" }
    }
  }
}
```

## systemd

Packaged paths: `/usr/lib/video-work-api`, `/etc/video-work-api/config.env`,
`/var/lib/video-work-api`. The unit is installed but never enabled automatically:

```bash
sudo systemctl start video-work-api.service
sudo systemctl stop video-work-api.service
```

## Development

```bash
cargo test
cargo build --release
bash -n scripts/vwactl .agents/skills/video-work-api/scripts/health-check.sh
```

Layout:

```
src/                 # Rust library + vwactl binary
  main.rs            # CLI (init/setup/serve/…)
  lib.rs             # modules: config, database, studio, http, mcp, …
scripts/             # vwactl wrapper, CosyVoice/FunClip helpers
static/              # browser UI
vendor/              # CosyVoice + FunClip git submodules
```

Model weights, reference voices, generated audio, databases, tokens, and
environment files are deliberately excluded from Git.
