# Authenticated API and HTTP MCP

All REST errors use `{"error":{"code":"...","message":"..."}}`. Unsafe browser
requests must include an `Origin` whose host exactly matches `Host`.
Authentication uses an opaque HttpOnly, SameSite=Strict cookie (`vwa_session`);
never log or persist its plaintext value.

## REST

- `GET /api/status` is public and never loads the model. Includes `product`,
  `mcp`, and `funclip_ready`.
- `POST /api/setup` accepts `token` and `password`; available once.
- `POST /api/auth/login` accepts `password`; `POST /api/auth/logout` signs out.
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

## HTTP MCP (`POST /mcp`)

Requires `Authorization: Bearer <VWA_MCP_TOKEN>` and is exempt from same-origin
when the bearer token is valid. JSON-RPC 2.0 methods: `initialize`,
`notifications/initialized`, `tools/list`, `tools/call`.

Tools:

| Tool | Purpose |
|------|---------|
| `get_status` | Service / model / FunClip status |
| `list_speakers` | Speakers and profiles |
| `create_speaker` | `{name}` |
| `delete_speaker` | `{speaker_id}` |
| `add_voice_profile` | `{speaker_id, style_name, prompt_text, audio_path, confirm_rights}` — path under `VWA_REFERENCE_INPUT_DIR` |
| `delete_voice_profile` | `{profile_id}` |
| `generate_speech` | `{speaker_id, profile_id, target_text, speed?}` — returns local `audio_path` |
| `get_generation` | `{generation_id}` |
| `extract_video_subtitles` | `{video_path}` under `VWA_VIDEO_INPUT_DIR` |

Do not bypass consent, authentication, file-size, duration, transcript,
ownership, path sandbox, or same-origin checks in integrations.
