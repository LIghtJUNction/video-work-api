---
name: video-work-api
description: This skill should be used when the user asks to "install Video Work API", "video work api", "视频工作 API", "import voice references", "start the voice cloning service", "extract video subtitles", "MCP video tools", "generate cloned speech", or "troubleshoot port 7860".
---

# Video Work API · 视频工作 API

Confirm that every reference voice has explicit informed consent. Never expose,
copy, or commit reference audio, transcripts, generated speech, databases,
passwords, setup tokens, MCP tokens, model weights, or environment files.

## Choose the operation

1. Read [references/operations.md](references/operations.md) for installation,
   initialization, model download, FunClip, folder import, service operation,
   paths, MCP, and troubleshooting.
2. Read [references/api.md](references/api.md) when integrating the authenticated
   REST API or HTTP MCP tools.
3. Run [scripts/health-check.sh](scripts/health-check.sh) for a read-only local
   service check. Pass a base URL as its only optional argument.

## Models and tools

- Voice clone: `FunAudioLLM/Fun-CosyVoice3-0.5B-2512` at revision
  `29e01c4e8d000f4bcd70751be16fa94bf3d85a18` (CosyVoice3, not Qwen3-TTS).
- Subtitles: FunClip stage-1 ASR via `vendor/FunClip` (FunASR Paraformer) with
  precise timestamps in SRT segments.

Prefer `vwactl` over ad-hoc filesystem edits. Keep the systemd unit disabled
unless a user explicitly requests boot enablement. Do not modify firewall rules
without explicit authorization. Treat port 7860 on `0.0.0.0` as LAN exposure;
recommend trusted-subnet filtering and HTTPS plus access control for untrusted
networks. MCP clients must send `Authorization: Bearer <VWA_MCP_TOKEN>`.
