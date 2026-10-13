# Snapper

A fast, free, cross-platform screenshot and annotation tool written in Rust. Snapper aims to replace Greenshot and ShareX on Windows, macOS and Linux with one memory-safe, single-binary app.

> Status: pre-alpha. The workspace compiles, but features are being built from the [roadmap](#roadmap). See the pinned roadmap issue.

## Vision

Greenshot is loved but Windows-only and slow to change. ShareX is powerful but Windows-only and overwhelming. Snapper keeps Greenshot's simplicity, adds ShareX's power features behind sensible defaults, and runs the same everywhere. It also takes privacy seriously: secrets, emails and tokens can be redacted automatically before an image ever leaves your machine.

## Features (planned for v1)

- Region, window and full-screen capture, across multiple monitors with mixed HiDPI scaling
- Scrolling capture for long pages and documents
- Global hotkeys and a tray (menu bar) app
- Annotation editor: arrows, boxes, ellipses, text, highlighter, blur and pixelate redaction, step numbers, crop
- Copy to clipboard, save with templated filenames such as `{date}_{time}_{app}.png`
- OCR of any region, copied as text
- Screen recording to GIF and MP4
- Upload targets via plugins: S3, Cloudflare R2, Imgur and custom HTTP
- Automatic redaction of secrets, API keys, emails and card numbers
- A scriptable CLI (`snapper capture region --out shot.png`)
- Settings UI backed by a plain TOML file
- Signed installers for Windows (MSI), macOS (notarised DMG) and Linux (AppImage, deb, rpm, Flatpak)

## Architecture

```
crates/
  snapper-core     shared types, settings, filename templates
  snapper-capture  platform capture backends (xcap, Wayland portal)
  snapper-editor   annotation document model and CPU renderer
  snapper-upload   upload target plugin interface and built-in targets
  snapper-cli      the `snapper-cli` binary
  snapper-app      the egui desktop app, tray and hotkeys
```

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and the ADRs in [docs/adr](docs/adr).

For setup and development, see the [user guide](docs/guide/README.md), [contribution guide](CONTRIBUTING.md), [Code of Conduct](CODE_OF_CONDUCT.md), and [platform permission notes](docs/platforms/).

## Roadmap

- **M0 Skeleton:** workspace, CI config, settings, logging
- **M1 MVP:** capture, editor basics, clipboard, save, tray, hotkeys, CLI
- **M2 Parity:** scrolling capture, OCR, recording, uploads, redaction, packaging
- **M3 Beyond:** auto-redaction, plugin SDK, workflows, accessibility polish

## Building

```
cargo check --workspace
cargo run -p snapper-app
```

## Licence

MIT. See [LICENSE](LICENSE).
