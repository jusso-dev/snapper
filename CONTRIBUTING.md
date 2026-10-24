# Contributing

Thanks for helping with Snapper. Issues and pull requests are welcome.

## Setup

Install Rust with the stable toolchain in `rust-toolchain.toml` (1.81 or newer, plus rustfmt and clippy). From the repository root:

```sh
cargo test --workspace
```

Linux OCR needs the Tesseract library and `--features tesseract` on `snapper-ocr`. The default build does not link it. macOS OCR uses Vision. Windows OCR uses `Windows.Media.Ocr` and is compiled only on Windows.

MP4 tests call `ffmpeg` when it is on `PATH` and skip the playback encode when it is not.

## Before a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Do not commit secrets, tokens, or certificate material. Upload secrets belong in the keychain, not in `config.toml`.

## Golden images

The editor renderer has golden PNGs under `crates/snapper-editor/tests`. A change to drawing should update those fixtures in the same commit and say why the pixels changed. Do not refresh them for an unrelated edit.

## Issues

Each engineering issue lists its dependencies and the commands to run. Start once those dependencies are done. Do not close an issue from a commit message unless the change actually meets the acceptance criteria, including the manual checks in `docs/VALIDATION.md`.

## Conduct

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
