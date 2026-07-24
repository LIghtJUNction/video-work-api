# Authenticated API and HTTP MCP

All REST errors use `{"error":{"code":"...","message":"..."}}`. Unsafe browser
requests must include an `Origin` whose host exactly matches `Host`.
Authentication uses an opaque HttpOnly, SameSite=Strict cookie (`vwa_session`);
never log or persist its plaintext value.

## REST

- `GET /api/status` is public and never loads the model. Includes `product`,
  `mcp`, `funclip_ready`, `video_editor`, and render queue state.
- `POST /api/setup` accepts `token` and `password`; available once.
- `POST /api/auth/login` accepts `password`; `POST /api/auth/logout` signs out.
- `POST /api/auth/mcp-token` returns the active env-override or persistent-file MCP token only to an
  authenticated, same-origin admin session. The response is never cacheable.
- `GET /api/auth/passkeys` lists public passkey metadata for the signed-in admin.
- `POST /api/auth/passkeys/register/start` accepts `{name}` and
  `POST /api/auth/passkeys/register/finish` completes browser enrollment. Both
  require an existing password-authenticated `vwa_session`.
- `DELETE /api/auth/passkeys/{id}` removes a passkey and requires authentication.
- `POST /api/auth/passkeys/login/start` and
  `POST /api/auth/passkeys/login/finish` perform passwordless browser login.
  WebAuthn requires an HTTPS domain, except that `http://localhost:<port>` is
  supported for local development. IP-literal origins are not supported. The
  admin password remains enabled as a recovery login.
- `POST /api/model/download` starts the fixed, pinned CosyVoice3 download in a
  single background task; `GET /api/model/download` returns its safe status.
  Both require an authenticated session. Expect roughly 10 GB of network and
  disk use, and install the Hugging Face CLI first.
- `GET /api/speakers` lists the private voice library.
- `POST /api/speakers` accepts JSON `name`.
- `DELETE /api/speakers/{id}` requires profiles to be deleted first.
- `POST /api/speakers/{id}/profiles` is multipart with `audio`, `style_name`,
  `prompt_text`, and `consent=true`.
- `DELETE /api/profiles/{id}` deletes an unused profile.
- `POST /api/generations` accepts JSON `speaker_id`, `profile_id`, `target_text`,
  and `speed` from 0.75 through 1.25.
- `GET /api/generations/{id}/audio` returns an authenticated WAV response.
- `POST /api/videos/subtitles` accepts JSON `video_path` relative to
  `VWA_VIDEO_INPUT_DIR` and returns `{segments, srt}` with precise timestamps.
- `POST /api/editor` accepts the same action object as the `video_editor` MCP
  tool. It requires an authenticated, same-origin administrator session.
  `write_file` is limited to `project.vpe`, requires `expected_revision`, and
  stages and flushes content before creating a durable database write intent.
  Recovery forward-completes interrupted publication under the project lock;
  history is create-only and hash-named, and the revision advances only after
  both history and current content are durable.
- `GET /api/editor/events` is the bounded, authenticated, same-origin SSE feed
  used by `/editor`. It accepts session cookies only (no bearer or query
  secrets), starts with a sanitized project/job snapshot, then emits
  `projects_changed`, `project_changed`, `job_changed`, and heartbeat events.
  Render jobs use the same public projection as `get_job`; absolute paths,
  executor details, and raw errors are excluded.
- Render submission, lookup, and cancellation are available only through the
  `video_editor` actions `export`, `get_job`, and `cancel_job`. Direct
  `/api/renders` submission is disabled.
- Renderers produce project deliverables only in a private per-job
  `attempt-1/` directory. A verified job enters a durable, non-claimable
  publication state before any project export changes. Publication is ordered,
  descriptor-confined, fsynced, and no-replace. Recovery forward-completes the
  intent without rerendering. Existing destinations are accepted only when
  size and SHA-256 match; conflicts preserve existing bytes and leave the job
  running with an operator-visible publication block. Cancellation no longer
  changes a job after durable publication begins. The intent CAS requires
  `cancel_requested=0`; if cancellation won first, the job atomically becomes
  canceled/cleanup-pending and no public file is written.

## HTTP MCP (`POST /mcp`)

Requires `Authorization: Bearer <VWA_MCP_TOKEN>` and is exempt from same-origin
when the bearer token is valid. JSON-RPC 2.0 methods: `initialize`,
`notifications/initialized`, `tools/list`, `tools/call`.

The advertised tool surface contains exactly one tool:

| Tool | Purpose |
|------|---------|
| `video_editor` | Sole MCP surface for status, speakers, consent-gated profiles, generation/subtitles, virtual projects, queued real-media sampling/trims/covers, gates, lifecycle, exports, job status, and cancellation |

Every project has one writable source of truth, `project.vpe`. Generated
`.history/`, `receipts/`, and `exports/` entries are visible but read-only.
Only the current validated revision can export. Export compiles a private
canonical EDL and binds the queued job to project ID, revision, and document
SHA-256. Every former direct MCP name, including status, speaker, profile,
generation, subtitle, and legacy render names, is neither advertised nor
callable; use the corresponding `video_editor` action.

VPE track declarations use English identifiers. `track main` remains the
single-track shorthand; named multi-track projects use `track main primary`.
Overlay tracks accept `type broll`, `type pip`, or `type effect`, for example
`track overlay product type broll`. Every main track is independently
continuous, starts at zero, and has the same duration. A variant declares at
least one optional payload: `subtitles`, `watermark`, or `cta`.

`extract_analysis_frames`, `analyze_safe_trims`, and `render_cover` enqueue real
media work in the same persistent single-worker queue as project exports.
Server-fixed FFmpeg/FFprobe processes run without a shell against immutable
private input snapshots. Sampling uses timestamps strictly before EOF and a
white timestamp/segment-ASR metadata bar; VLM analysis is not provided.
Safe-trim analysis detects real audio silence and uses either genuine supplied
word timestamps or genuine FunClip/FunASR token timestamps produced by
stage-1 ASR on the frozen input; it never interpolates timestamps. Subtitle
extraction returns `words` alongside segments and SRT. `render_cover` takes a
declared `variant_key`; pre-package `cover_jobs` maps every stable variant key
to one unique succeeded cover job ID. The server accepts no caller-provided
cover path/hash proof.
Read results through `get_job`; cancel through `cancel_job`.
`get_job` returns public state and project-relative results only; private
absolute log, snapshot, output, render-plan paths, and detailed internal errors
are never serialized; terminal errors use bounded public codes/messages.
Pre-package and acceptance take a succeeded `video_project` `job_id`, not a
caller-selected render report. The worker-owned DB attestation binds the
verified report/bundle/output/replay hashes and each variant's index, language,
aspect, watermark, CTA, unique path, and hash. Cover attestations additionally
bind CoverSpec and are hash-checked and decoded from actual distinct files.
Faststart is checked for Master and
every final variant.
`cleanup_intermediates` and `archive_completed_sources` default to dry-run;
actual changes require explicit `dry_run: false`. Cleanup is limited to
service-owned `cache/`, `proxies/`, and `passes/` trees and never removes the
project `.tmp/` tree. Archival requires the preserved four-piece export set,
the complete revision-scoped pre-render to acceptance receipt chain, and exact
PASS-report hash bindings. It transactionally moves only VPE-declared
`project/assets` sources with a durable journal while preserving exports.
Missing `VWA_RECEIPT_KEY_FILE` is reported as signing capability unavailable,
never as a fake receipt.

Do not bypass consent, authentication, file-size, duration, transcript,
ownership, path sandbox, or same-origin checks in integrations.
