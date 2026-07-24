# Video Work API agent guide

- Product: Video Work API (视频工作 API); crate: `video-work-api`; CLI: `vwactl`.
- Language: **Rust** with `src/` layout (`src/lib.rs` + `src/main.rs` → `vwactl`).
- Voice model: FunAudioLLM/Fun-CosyVoice3-0.5B-2512, not Qwen3-TTS.
  Inference helper: `scripts/cosyvoice_infer.py` (Python + vendored CosyVoice).
- Subtitles: FunClip (`vendor/FunClip`) stage-1 ASR with time-coded SRT segments.
- Translation: `google/madlad400-3b-mt` via `scripts/madlad_translate.py`;
  languages list puts English then Russian first for UI convenience.
- Service: `video-work-api.service`; default port: `7860`.
- MCP: `POST /mcp` with `Authorization: Bearer <VWA_MCP_TOKEN>`.
- Installed app/config/data paths: `/usr/lib/video-work-api`,
  `/etc/video-work-api/config.env`, `/var/lib/video-work-api`.
- Never commit voices, generated audio, model weights, SQLite files, tokens,
  passwords, environment files, caches, or virtual environments.
- Do not enable the service or edit firewall rules automatically.
- Preserve exact reference transcripts, explicit rights confirmation, opaque
  filenames, no-overwrite publication, authentication, path sandboxes, and
  same-origin checks (MCP bearer is the agent path).
- Run: `cargo test`; `cargo build --release`; `bash -n scripts/vwactl
  .agents/skills/video-work-api/scripts/health-check.sh`.
