# Security, consent, and responsible use

Use this software only for a voice you own or when the identifiable speaker has
given explicit, informed permission for cloning and the intended distribution.
Do not impersonate a person, evade consent, deceive listeners, conduct fraud,
or remove disclosures required by law or platform policy.

Voice recordings are sensitive biometric-like personal data. Keep reference
audio, transcripts, generated speech, the SQLite database, setup token, MCP
token, and administrator credentials out of Git and backups shared with third
parties. Deletion from the UI does not erase copies in external backups.

The service binds to `0.0.0.0:7860` by default for LAN use. The password protects
browser REST APIs; MCP uses a separate bearer token (`VWA_MCP_TOKEN`). Plain
HTTP does not protect credentials or audio in transit. Limit firewall access to
a trusted subnet. For any untrusted network, put the service behind an
authenticated HTTPS reverse proxy or bind it to `127.0.0.1`. Never expose it
directly to the public Internet.
Browser microphone capture is available only in a secure context: HTTPS or
localhost. Optional `VWA_SSL_CERTFILE` and `VWA_SSL_KEYFILE` settings can serve
HTTPS directly, but certificate issuance and renewal remain the operator's
responsibility.

Path sandboxes restrict agent-facing files: videos under `VWA_VIDEO_INPUT_DIR`,
MCP reference audio stays under `VWA_REFERENCE_INPUT_DIR`. Video Project Editor
assets stay under each project's private `assets/` directory. Render callers
cannot supply a command or renderer flags: export compiles the validated VPE
revision into a canonical private bundle and binds the job to project ID,
revision, document SHA-256, renderer hash, and regular-file asset identities.
Direct legacy XRY REST/MCP render submission is disabled.
The database permits only one running render, and an OS-released exclusive lease
prevents a second process from recovering or driving the queue. The service
unit keeps `/srv/0.参考` and `/srv/3.成品` read-only. Do not point these
directories at world-writable shared storage without additional controls.

Video Project Editor data is confined to `VWA_VIDEO_PROJECTS_DIR`. Project
slugs and virtual paths reject roots, parent/current components, and symlinks.
`project.vpe` is the only writable virtual file; `.history`, `receipts`, and
`exports` are read-only through the editor. Writes use optimistic revisions,
private same-filesystem temporary files, `fsync`, and atomic rename. Export is
rejected unless the on-disk hash matches the current database revision and that
exact revision has passed VPE parsing and timeline validation.

Sampling and cover actions produce argument/layout plans only. Video inputs are
resolved under `VWA_VIDEO_INPUT_DIR`; no caller-controlled shell command is
executed. Quality gates fail closed when required subtitle, variant,
four-piece, or MP4 faststart evidence is missing. Lifecycle actions default to
dry-run, accept only explicit allowlisted project-relative targets, reject
symlink components, and never recursively delete a caller-selected directory.
Archival requires the exact MP4/TXT/JPG/`-cover-original.png` set, an acceptance
PASS report, and a valid HMAC-SHA256 receipt binding each artifact hash.

Receipt keys are read only from `VWA_RECEIPT_KEY_FILE` and are never returned
through REST or MCP. Receipts use canonical parameters, SHA-256 hashes, an
executor hash, a previous-receipt link, and HMAC-SHA256. A missing key disables
signing; it does not downgrade to an unsigned or fabricated signature.

Security reports should be sent privately through GitHub's security advisory
feature. Do not include real voices, tokens, passwords, or generated private
audio in a public issue.
