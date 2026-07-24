<div align="center">

# Video Work API

**A self-hosted voice and subtitle workspace for people, applications, and AI agents.**

Authorized zero-shot voice cloning with CosyVoice3, time-coded video subtitles
with FunClip, and a browser studio—served by one authenticated Rust service.

[English](README.md) · [简体中文](README.zh-CN.md)

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-9b4f3b.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/Rust-1.88%2B-202421.svg?logo=rust)](Cargo.toml)
[![Platform: Linux](https://img.shields.io/badge/platform-Linux-d8dfcf.svg?logo=linux&logoColor=202421)](#requirements)
[![AUR: video-work-api-git](https://img.shields.io/aur/version/video-work-api-git?label=AUR&color=c9775e)](https://aur.archlinux.org/packages/video-work-api-git)
[![MCP: HTTP](https://img.shields.io/badge/MCP-HTTP-62675f.svg)](#rest-api-and-http-mcp)

[Quick start](#quick-start) · [Highlights](#why-video-work-api) · [MCP tool](#the-video_editor-mcp-tool) · [Security](#security-and-responsible-use) · [Contributing](CONTRIBUTING.md)

</div>

![Video Work API workflow: authorized voice and sandboxed video inputs flow through the Rust service to CosyVoice3, FunClip, the browser studio, REST API, and HTTP MCP.](static/assets/video-work-api-overview.svg)

> [!IMPORTANT]
> Clone or publish a voice only with the identifiable speaker's explicit,
> informed permission. Voice cloning can enable impersonation and fraud. Read
> the [security and responsible-use policy](SECURITY.md) before importing audio.

## Why Video Work API?

- **One private workspace, three ways to use it.** Work in the bilingual web
  studio, integrate the session-authenticated REST API, or give a trusted AI
  agent one consolidated bearer-authenticated `video_editor` MCP tool.
- **Voice references become reusable assets.** Organize clips by speaker and
  style, rename them as a library evolves, and generate individual WAV files
  from one or many lines of text.
- **Video to usable SRT in the same service.** Submit sandboxed paths or browser
  uploads and receive time-coded segments plus a downloadable SRT file.
- **Built around consent and local ownership.** Exact transcripts, explicit
  rights confirmation, private tokens, and path sandboxes are enforced rather
  than left to client convention.
- **Useful with or without an agent.** It is a practical local studio first;
  MCP adds automation without replacing the browser workflow.

## What can I use it for?

| Use case | What Video Work API provides |
|---|---|
| Authorized creator voiceovers | Reuse distinct speaking styles and generate downloadable WAV files |
| Localization and accessibility | Produce draft narration and time-coded SRT subtitles locally |
| Private media workflows | Keep references, outputs, metadata, and model files on your own machine |
| Agent-assisted production | Let Codex or another MCP client list voices, generate speech, and extract subtitles |
| Small-team voice libraries | Manage speakers and profiles in a bilingual, password-protected browser studio |

This project is deliberately focused: it does not perform FunClip stage-2
clipping or make consent decisions for you. Machine translation of plain text
and SRT is available via MADLAD-400-3B (`POST /api/translate`).

## How it works

The Rust service owns authentication, metadata, consent checks, filesystem
boundaries, and publication. Python helpers are used only for the vendored
CosyVoice and FunClip inference runtimes.

1. Add an authorized 5–30 second reference clip and its exact transcript, or
   place/upload a video inside the allowed input boundary.
2. CosyVoice3 generates speech from a selected profile; FunClip **stage-1 ASR
   only** produces time-coded subtitle segments and SRT.
3. Use the result through the browser studio, authenticated REST API, or HTTP
   MCP. Generated audio is returned as a local path to agents, never base64.

## Quick start

### Arch Linux (AUR)

[`video-work-api-git`](https://aur.archlinux.org/packages/video-work-api-git)
builds the current Git revision and installs the service layout.

```bash
paru -S video-work-api-git
sudo vwactl setup
sudo vwactl init
sudo vwactl model download                         # CosyVoice3 voice, ~10 GB
sudo vwactl model download --kind translation      # MADLAD-400-3B, ~12 GB
sudo systemctl start video-work-api.service
```

Open `http://localhost:7860` and finish setup with the one-time token printed by
`sudo vwactl init`. The package installs the systemd unit but **does not enable
or start it automatically** and does not change firewall rules.

### Build from source

```bash
git clone --recurse-submodules https://github.com/LIghtJUNction/video-work-api.git
cd video-work-api
cargo build --release
./scripts/vwactl setup
./scripts/vwactl init
./scripts/vwactl model download                         # CosyVoice3 voice, ~10 GB
./scripts/vwactl model download --kind translation      # MADLAD-400-3B, ~12 GB
./scripts/vwactl serve
```

Then open `http://localhost:7860`. `setup` creates a Python virtual environment
for the inference runtimes; the API server itself is the Rust `vwactl` binary.

<details>
<summary><strong>First login, model download, and passkeys</strong></summary>

`vwactl init` creates private data directories, SQLite state, a persistent
mode-0600 MCP token, and a one-time web setup token. Use the setup token to
create an administrator password of at least 12 characters. The model download
is fixed to the repository and revision listed below, reuses the Hugging Face
Hub cache, and requires the `hf` CLI.

After password login, you can register a passkey. WebAuthn requires an HTTPS
domain, except for `http://localhost:<port>` during local development; literal
IP origins are not supported. The administrator password remains a recovery
login. Reset it interactively with `./scripts/vwactl passwd` (source) or
`sudo vwactl passwd` (package).

</details>

## Browser studio

The English/Chinese studio provides:

- one-time setup, administrator sessions, and optional passkey login;
- speaker and voice-profile creation, rename, and guarded deletion;
- browser microphone recording or audio upload with exact transcript and
  explicit consent confirmation;
- batch speech generation (one WAV per non-empty line), playback, retry, and
  human-readable downloads;
- batch subtitle extraction from sandboxed paths and local uploads (up to 2 GiB
  each), with progress, retry, preview, and SRT download;
- authenticated model-download status and a **Copy agent prompt** flow for
  configuring Codex or Claude Code without exposing the token before login.

Microphone capture requires a secure browser context: HTTPS or localhost.

## REST API and HTTP MCP

These are intentionally separate trust paths:

| Interface | Intended client | Authentication | Browser origin policy |
|---|---|---|---|
| REST under `/api/*` | Web studio and application integrations | Public status, one-time setup, password login, and passkey-login entry points; authenticated endpoints use an opaque `HttpOnly`, `SameSite=Strict` admin session | Unsafe requests require the `Origin` host and port to match `Host` |
| HTTP MCP at `POST /mcp` | Trusted AI agents | `Authorization: Bearer <VWA_MCP_TOKEN>` | Bearer-authenticated MCP is exempt from browser same-origin checks |

REST covers setup, login/logout, passkeys, model download, speaker/profile
management, speech generation, authenticated WAV retrieval, and subtitle
extraction. See the live `/docs` page after starting the service for a compact
endpoint reference.

### The `video_editor` MCP tool

| Tool | Purpose |
|---|---|
| `video_editor` | The sole MCP tool: speakers, consent-gated voice profiles, speech/subtitles, virtual project editing, real queued media analysis/covers, quality gates, lifecycle, and queued exports |

The production human workbench is available at `/editor` after administrator
login. It uses the approved native code-workbench shell with a live project
inspector: multiple virtual project roots, expandable trees, tabs, line-numbered
plain-text editing, parsed timeline/marker/transition/variant/gate context, and
the sanitized render queue remain visible together. `project.vpe` is the only
writable file. Saves use `expected_revision`; a dirty buffer is never replaced
by a remote update. The authenticated, same-origin
`GET /api/editor/events` SSE stream sends initial project/job state and
sanitized revision/job changes without bearer/query tokens or private paths.
The UI falls back to `get_job` polling while reconnecting.

Each project is a virtual folder whose only writable file is `project.vpe`.
Generated `.history/`, `receipts/`, and `exports/` folders stay visible and
read-only. Writes require `expected_revision`, are staged in a private file,
flushed, and registered as a durable write intent before publication.
Interrupted writes are forward-completed under the project lock from the
verified staged, history, or current copy. History revisions use create-only
hash-named files, and the database revision advances only after both history
and `project.vpe` are durable. Only the current validated revision can export.

The same tool exposes queued `extract_analysis_frames`, `analyze_safe_trims`,
and `render_cover` actions. They use the persistent FIFO single-worker queue,
fixed server-side FFmpeg/FFprobe, private input snapshots, cancellation,
timeouts, and process-group recovery. Frame extraction defaults to 12 frames
at 360x640, keeps every timestamp strictly before EOF, and adds a white
timestamp/segment-ASR metadata bar. It does not perform VLM analysis.
Safe-trim analysis derives silence from the real audio stream and invokes the
conservative trim boundary algorithm. If the caller supplies no word
timestamps and FunClip is configured, fixed stage-1 ASR runs against the
frozen input and supplies genuine token timestamps; none are interpolated.
Subtitle extraction returns those word timestamps alongside segments and SRT.
Cover rendering takes a declared stable variant key, binds the current project
revision, document, VariantSpec, and CoverSpec, decodes both generated images,
then publishes immutable `<variant-key>-cover-original.png` and
`<variant-key>.jpg`. Pre-package accepts only succeeded cover job IDs keyed by
variant; caller-provided paths or hashes are not evidence.
Renderers write only to a private `attempt-1/` directory. After verification,
the worker durably records an ordered publication intent containing every
source/destination path, size, and SHA-256. It then publishes through pinned
project/export directory descriptors using fsynced temporary files and
no-replace renames. Restart recovery completes an existing intent without
rerendering; an interrupted attempt without an intent is removed and requeued.
Cancellation and publication use one database CAS: cancellation committed
before the intent publishes zero files and becomes cleanup-pending, while an
existing intent makes later cancellation a no-op and completes successfully.
An identical existing file is accepted, while any content conflict leaves the
job running and publication-blocked, preserves existing exports, and stops
further claims for operator inspection. Cover pairs and all video/report files
use this same all-files protocol. Private attempts are cleanup-pending only
after the database commits success.
`get_job` exposes only public job state and project-relative deliverables; it
never returns private snapshot/log/plan paths, executor arguments, or internal
errors; failures use bounded public codes and generic messages. Completed
jobs discard frozen source media and private replay media while retaining the
canonical plan, verified report, log, and public analysis outputs.
Cleanup and archival default to `dry_run: true`; destructive execution requires
an explicit `dry_run: false`, sandboxed service-owned `cache/`, `proxies/`, or
`passes/` paths, and all server-side gates; project `.tmp/` is never a cleanup
target. Source archival verifies the complete revision-scoped receipt chain,
moves only VPE-declared `project/assets` sources through a durable staging
journal, and leaves every export in place. Acceptance requires a valid `ftyp`
box and `moov` before `mdat` on the Master and every variant. Pre-package and
acceptance require the exact succeeded `video_project` job ID. The worker
persists a DB attestation only after independently hashing all primary/replay
outputs and binding each variant's index, language, aspect, watermark, CTA,
path, and hash. Callers cannot supply a report path or replay assertions.
Variant output paths and artifacts must be distinct, and declared copy/cover
hashes are recomputed from actual files. Immutable PASS reports and signed receipts live
under `receipts/rev-<n>/<phase>/`; receipts bind the exact report hash, project,
revision, document, gate results, inputs, outputs, executor, and prior receipt.
Failed reports are never signed or published to the canonical PASS path. If
`VWA_RECEIPT_KEY_FILE` is absent, signing reports capability unavailable
without consuming that path, so the same revision can be retried.

The all-English VPE document describes canvas, sources, tracks, clips, cuts,
holds, transitions, markers, variants, and gates:

```text
project "Aurora Launch" {
  canvas 1080x1920 @ 30fps
  source host = "assets/host.mp4"
  source detail = "assets/detail.mp4"
  timeline {
    track main primary {
      clip host source 00:00:00.000..00:00:03.120 at 00:00:00.000
      cut at 00:00:03.120
      transition cross_dissolve at 00:00:03.120 duration 00:00:00.200
      clip host source 00:00:03.120..00:00:06.200 at 00:00:03.120
    }
    track overlay product type broll {
      clip detail source 00:00:00.000..00:00:01.000 at 00:00:02.000
    }
    track overlay graphics type effect {
      effect vignette at 00:00:02.800..00:00:03.200
    }
  }
  marker "Opening hook" at 00:00:03.000
  variant "ZH-EN" aspect 9:16 subtitles "subs.zh-en.ass" watermark "brand/logo.png" cta "Learn more"
  gate pre_render require input_manifest, continuous_timeline, opening_hook, subtitle_overflow
  gate pre_package require output_specifications, cover_match, copy_consistency
  gate acceptance require deterministic_replay, faststart
}
```

`export` compiles the exact validated revision into an immutable canonical EDL
bundle. The persistent single-concurrency worker verifies the document,
renderer, and frozen project-asset hashes again, renders `master.mp4`, then derives
the requested aspect/subtitle/watermark/CTA variants under the project's
read-only virtual `exports/` tree. Named main tracks render as
duration-preserving layers with mixed audio, and variant names use stable
index/language/aspect keys. Unknown effects or transitions, symlinked assets,
and stale revisions fail closed. The former
direct XRY render REST/MCP submission surface is disabled.

After administrator login, **Copy agent prompt** provides setup instructions for
both Codex and Claude Code. Project scope uses `.codex/config.toml` for Codex or
`.mcp.json` for Claude Code; user/global scope uses each client's user-level MCP
configuration. Treat every resulting configuration as a secret because it
contains the static bearer token.

<details>
<summary><strong>MCP token lifecycle</strong></summary>

The token persists across restarts and upgrades at
`$VWA_DATA_DIR/mcp-token` unless `VWA_MCP_TOKEN_FILE` changes it;
`VWA_MCP_TOKEN` is a higher-priority compatibility override. Rotate deliberately
with `vwactl mcp-token rotate`, restart the service, sign in, copy the new agent
prompt, replace the client's configuration, then restart or open a new client
session and verify the live tools. A restart alone cannot update a client's
static header.

</details>

## Requirements

- Linux, **Rust 1.88+**, Python 3.10+, `uv`, FFmpeg, SoX, Git LFS, and the
  Hugging Face CLI (`hf`, provided for example by `python-huggingface-hub`)
- NVIDIA CUDA recommended for CosyVoice inference; CPU works but is slow
- Roughly 10 GB of network and disk capacity for the pinned CosyVoice3 snapshot
  and runtime environment
- Additional FunASR models downloaded on first subtitle extraction

### Fixed model and runtime scope

- Voice model: [`FunAudioLLM/Fun-CosyVoice3-0.5B-2512`](https://huggingface.co/FunAudioLLM/Fun-CosyVoice3-0.5B-2512)
- Revision: `29e01c4e8d000f4bcd70751be16fa94bf3d85a18`
- Inference runtime: vendored [`FunAudioLLM/CosyVoice`](https://github.com/FunAudioLLM/CosyVoice) (CosyVoice3, not Qwen3-TTS)
- Subtitles: vendored [`modelscope/FunClip`](https://github.com/modelscope/FunClip), stage-1 FunASR Paraformer only

## Security and responsible use

- Obtain explicit, informed permission for the voice and its intended use.
- Preserve the exact reference transcript; do not bypass `confirm_rights`.
- Keep voices, transcripts, generated media, SQLite data, tokens, credentials,
  model weights, caches, and environment files out of Git.
- MCP reference audio and videos are restricted to their configured input
  directories; symlinks and unsafe paths are rejected.
- The default `0.0.0.0:7860` bind is LAN-visible. For untrusted networks, bind
  to loopback or use an authenticated HTTPS reverse proxy and trusted-subnet
  filtering. Never expose this service directly to the public Internet.
- Installation does not enable the service or modify firewall rules.

Read [SECURITY.md](SECURITY.md) for the full threat model and reporting path.

<details>
<summary><strong>Configuration and installed paths</strong></summary>

Configuration uses the `VWA_*` prefix. Common settings include
`VWA_DATA_DIR`, `VWA_MODEL_DIR`, `VWA_COSYVOICE_ROOT`, `VWA_FUNCLIP_ROOT`,
`VWA_HOST`, `VWA_PORT`, `VWA_VIDEO_INPUT_DIR`, `VWA_REFERENCE_INPUT_DIR`,
`VWA_VIDEO_PROJECTS_DIR`,
`VWA_RECEIPT_KEY_FILE`,
`VWA_XRY_TASK_ROOT`, `VWA_XRY_SOURCE_ROOT`, `VWA_XRY_RENDERER`, `VWA_XRY_PYTHON`,
`VWA_RENDER_TIMEOUT`, `VWA_VIDEO_PROJECT_RENDERER`,
`VWA_VIDEO_PROJECT_PYTHON`, `VWA_VIDEO_PROJECT_RENDER_TIMEOUT`,
`VWA_MCP_TOKEN_FILE`, and optional `VWA_SSL_CERTFILE` / `VWA_SSL_KEYFILE`.
The certificate and key paths are validated together as regular, non-symlink
files, but they do not enable built-in HTTPS; the service still serves HTTP.
Use an HTTPS reverse proxy for secure non-localhost or LAN browser contexts.
See [`config.env.example`](config.env.example) for the authoritative defaults.

Packaged paths:

| Purpose | Path |
|---|---|
| Application | `/usr/lib/video-work-api` |
| Configuration | `/etc/video-work-api/config.env` |
| Private data | `/var/lib/video-work-api` |
| Service | `video-work-api.service` |

Use `vwactl paths` and `vwactl status` to inspect the effective source-install
configuration. Use `systemctl start`, `stop`, or `restart` explicitly for the
packaged unit; do not assume it is enabled.

For XRY rendering, deploy the receipt key through the dedicated read-only
group. Do not add the service to a broad operator group:

```bash
sudo groupadd --system --force xry-render
sudo usermod --append --groups xry-render video-work-api
sudo usermod --append --groups xry-render xry
sudo python /srv/xry/.agents/skills/xry-video-acceptance/scripts/generate_receipt_key.py
sudo chown root:xry-render /etc/xry/render-receipt.hmac.key
sudo chmod 0640 /etc/xry/render-receipt.hmac.key
```

The packaged unit declares `SupplementaryGroups=xry-render`; its install hook
creates the group, enrolls `video-work-api` and an existing `xry` account, and
normalizes an existing receipt key to `root:xry-render` mode `0640`. It never
enables or starts the service.

Video Project exports enter a persistent FIFO queue. A database constraint and
exclusive worker lease keep the global running count at one across processes.
Cancellation, timeout, renderer errors, and service shutdown terminate the
whole renderer process group and release the slot. FIFO order uses a durable
monotonic enqueue sequence, not wall-clock timestamps or UUIDs. Each job owns a
canonical EDL and frozen asset snapshot bound to project/revision/document
hashes. The renderer performs an independent replay, and success requires
byte-identical output hashes. After an unclean restart, a durable launch
handshake closes the spawn-to-database PID window. Once PID/starttime is
persisted, recovery independently verifies `/proc` process-group and fixed
executor/job cmdline identity even if the handshake is missing or corrupt.
Before signaling it stores a durable recovery intent; ambiguous identity stays
`running` with claims blocked. Terminal jobs similarly retain a durable
cleanup-pending flag until descriptor-confined cleanup completes.

</details>

## Project layout

```text
src/                 Rust library and vwactl service/CLI
scripts/             Setup wrapper and inference helpers
static/              Bilingual browser studio and assets
vendor/              CosyVoice and FunClip submodules
systemd/             Packaged service unit
packaging/aur/       AUR VCS package files
tests/               API, CLI, and browser-contract tests
```

## Documentation and development

- [Security and responsible use](SECURITY.md)
- [Configuration example](config.env.example)
- [Packaging and AUR notes](packaging/README.md)
- [Contributing guide](CONTRIBUTING.md)
- Live API reference: `/docs` on a running instance

```bash
cargo test
cargo build --release
bash -n scripts/vwactl .agents/skills/video-work-api/scripts/health-check.sh
```

Tests use fake inference and temporary directories; they do not require model
downloads. Please keep English and Simplified Chinese README sections in sync.

## License

Video Work API is licensed under [Apache License 2.0](LICENSE). Vendored model
code and model artifacts retain their respective upstream licenses and terms.
