# Contributing

Keep changes narrow, preserve the consent and filesystem safety boundaries, and
never add real reference voices or model weights. Use English for code,
identifiers, and comments; update both README languages when behavior changes.

This project is a **Rust** crate (`src/` layout). CosyVoice and FunClip remain
vendored Python runtimes invoked as subprocesses.

Run `cargo test`, `cargo build --release`, and shell syntax checks before
opening a pull request. Tests must use temporary directories and fake
inference—never download models or contact external services.
