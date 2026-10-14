# Snapper

A fast, free, cross-platform screenshot and annotation tool written in Rust. Snapper aims to replace Greenshot and ShareX on Windows, macOS and Linux with one memory-safe, single-binary app.

> Status: alpha. Capture, the annotation editor, the tray, hotkeys, the CLI, scrolling capture, OCR, GIF and MP4 recording, S3, R2, Imgur, custom HTTP uploads, local redaction, plugins, and workflows are implemented. Installers, paid code signing, and live screen-reader passes are documented and not yet verified. See [docs/VALIDATION.md](docs/VALIDATION.md).

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
  snapper-capture  platform capture backends (xcap, Wayland portal) and scroll stitching
  snapper-editor   annotation document model and CPU renderer
  snapper-ocr      platform OCR and local secret detection
  snapper-record   GIF (gifski) and MP4 (ffmpeg) recording
  snapper-upload   S3, R2, Imgur, custom HTTP, and external plugins
  snapper-cli      the `snapper-cli` binary
  snapper-app      the egui desktop app, tray and hotkeys
```

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and the ADRs in [docs/adr](docs/adr).

## Guides

- [Capture](docs/guide/capture.md), [editor](docs/guide/editor.md), [command line](docs/guide/cli.md)
- [Uploads](docs/guide/uploads.md) and [workflows](docs/guide/workflows.md)
- [Plugins](docs/plugins.md)
- [Windows](docs/platforms/windows.md), [macOS](docs/platforms/macos.md), [Linux X11](docs/platforms/linux-x11.md), [Linux Wayland](docs/platforms/linux-wayland.md)
- [Releasing](docs/RELEASING.md) and [accessibility](docs/accessibility.md)
- [Contributing](CONTRIBUTING.md) and the [code of conduct](CODE_OF_CONDUCT.md)

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
