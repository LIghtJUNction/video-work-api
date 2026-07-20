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

Path sandboxes restrict agent-facing files: videos under `VWA_VIDEO_INPUT_DIR`
and MCP reference audio under `VWA_REFERENCE_INPUT_DIR`. Do not point these
directories at world-writable shared storage without additional controls.

Security reports should be sent privately through GitHub's security advisory
feature. Do not include real voices, tokens, passwords, or generated private
audio in a public issue.
