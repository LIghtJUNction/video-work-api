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
./scripts/vwactl serve
```

In a packaged install, run administrative commands through `sudo vwactl`. The
wrapper re-executes setup, init, mcp-token, model, import, passwd, status, and paths as the dedicated
service account. Do not bypass that wrapper or create root-owned files in
`/var/lib/video-work-api`.

Run `vwactl init` once. It creates private data directories, the SQLite
database, and a mode-0600 one-time token. Print a pending token again with
`vwactl token` (or `vwactl token --raw` for scripts). Rotate with
`vwactl token create --rotate` before first web setup. Enter that token in the
web setup page and create a password of at least 12 characters. Successful setup
removes the token. Do not paste it into logs or chat.

If the administrator password is lost, reset it interactively from a terminal:

```bash
# Source installation
./scripts/vwactl passwd

# Packaged installation
sudo vwactl passwd
```

The command hides both password entries, requires at least 12 characters, and
signs out every existing web session. Registered passkeys and all voice and
generated data are preserved.

Run `vwactl model download` only with explicit approval for the multi-gigabyte
download. It shells out to the Hugging Face CLI (`hf download`), pins both the
repository and revision, requests only the files listed in
`scripts/download_model.py`, and reuses the Hub cache (`~/.cache/huggingface/hub`
or `HF_HUB_CACHE`). Install `python-huggingface-hub` (or another package that
provides `hf`) before downloading.

Start a foreground development service with `vwactl serve`. Defaults are
`0.0.0.0:7860` and `~/.local/share/video-work-api`. Inspect effective
locations with `vwactl paths` and configuration readiness with `vwactl status`.

Environment variables use the `VWA_*` prefix. See `config.env.example`.

`vwactl init` and `vwactl serve` create `$VWA_DATA_DIR/mcp-token` once with
mode 0600 when `VWA_MCP_TOKEN` is absent. The token persists across restarts and
upgrades and is never printed. `VWA_MCP_TOKEN_FILE` overrides its path;
`VWA_MCP_TOKEN` remains the highest-priority compatibility override. Use
`vwactl mcp-token ensure` to provision it explicitly and `vwactl mcp-token
rotate` for intentional rotation. After rotation, restart the service, sign in
as administrator, copy the new agent prompt, rerun the same project/global
installation branch to replace the client's static token, then restart/open a
new Codex session and verify live tools. Restarting alone is insufficient.

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

Clients POST JSON-RPC to `http://HOST:PORT/mcp` with `Authorization: Bearer
<token>`. After administrator login, **Copy agent prompt** returns a Codex-first
prompt containing the active token. It asks once whether to install into the
current trusted project's `.codex/config.toml` or global
`~/.codex/config.toml`, then verifies the live MCP connection. Put
agent-readable reference audio under `VWA_REFERENCE_INPUT_DIR` for
`add_voice_profile`.

The public MCP surface is the single `video_editor` tool. Agents list projects,
inspect their virtual trees, read and optimistically write `project.vpe`,
validate the current revision, then queue and observe exports through actions
on that tool. Project slugs and virtual paths are sandboxed; symlinks and parent
components are rejected.

The editor's media actions are persistent queued work.
`extract_analysis_frames`, `analyze_safe_trims`, and `render_cover` share the
single render worker, cancellation, timeout, process-group, and crash-recovery
path. They use fixed FFmpeg/FFprobe without a shell and immutable snapshots.
Sampling does not perform VLM analysis. Safe-trim analysis uses genuine
caller-supplied word timestamps, or configured FunClip stage-1 token
timestamps from the frozen input, and never interpolates them. Cover jobs bind
the current revision, stable variant key, VariantSpec, and CoverSpec; the
worker hash-checks and decodes both images. Pre-package accepts only
authoritative succeeded cover job IDs. Phase validation is
fail-closed, ordered
pre-render to pre-package to acceptance, and revision-scoped; failed reports
and signing-unavailable attempts do not consume the canonical PASS path.
Cleanup and archival are dry-run by default, reject symlinks
and arbitrary paths, and require explicit `dry_run: false` for changes.
Cleanup never targets project `.tmp/`. Archival requires the preserved
four-piece delivery, complete verified receipt chain, exact PASS-report
bindings, and transactionally moves only VPE-declared source assets while
leaving exports in place.
Configure an existing private HMAC key through `VWA_RECEIPT_KEY_FILE`; if it is
missing, receipt signing is unavailable and no substitute signature is made.

## systemd

The service is `video-work-api.service`. Use `systemctl status`, then
explicit `sudo systemctl start`, `stop`, or `restart`. Installation must not run
`enable`. Read logs with `journalctl -u video-work-api.service` while
avoiding output that contains private text or paths.

The render queue is persistent FIFO with a database-enforced global limit of one
running render and an exclusive worker lease. `video_editor` export snapshots a
canonical EDL bound to project ID, revision, and document SHA-256. Source,
subtitle, and watermark files are copied from verified regular descriptors
into a private frozen bundle. The worker verifies Python and renderer hashes,
renders all main tracks and variants, and performs an independent byte replay.
All renderer outputs remain under the private job `attempt-1/` directory until
verification has produced a durable publication intent. That intent binds the
job, project, revision, document, attestation/report, and an ordered manifest
of project-relative destinations with source size and SHA-256. Publication
pins the private job and project directories, copies to a private temporary
file, fsyncs it, uses a no-replace rename, and fsyncs the destination parent.
Matching existing content is an idempotent success; a mismatch remains running
and publication-blocked, stops further claims, and never removes or overwrites
the existing export. Startup recovers publication intents before orphan
renderers or new claims. A complete intent is forward-completed without
rerendering; a stale attempt without an intent is removed before requeue.
The publication-intent CAS and cancellation update are mutually exclusive:
cancel first yields cleanup-pending with zero public files, while intent first
rejects later cancellation and forward-completes publication.
Success, trusted attestation, and cleanup-pending are committed together, and
only then may cleanup remove the private attempt. This protocol applies to the
cover pair and every video/report file.
On graceful shutdown, a running
renderer receives TERM, then KILL after a bounded wait; its process group is
reaped and its job becomes terminal before the worker exits. A durable launch
handshake covers crashes before the database PID write. With a persisted PID,
recovery independently authenticates `/proc` starttime, process group, and the
fixed wrapper/job cmdline, stores a durable recovery intent before signaling,
then safely requeues. Ambiguity remains running and blocks claims. A new lease holder
safely requeues interrupted jobs after an unclean restart only after matching
the stored Linux PID plus `/proc` starttime and confirming that process group is
gone. Ambiguous cleanup stops the worker. Queue order is a durable monotonic
sequence. Legacy XRY database rows remain readable but fail closed when their
historical official-path invariants cannot be established.
Terminal transitions atomically set cleanup-pending. Startup and the worker
loop complete descriptor-confined cleanup before any next claim and clear the
flag only afterward; unsafe cleanup stops the worker. Frozen source/replay
media are removed while queued/running inputs and all project exports remain.

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
