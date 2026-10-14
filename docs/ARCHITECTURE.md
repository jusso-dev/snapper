# Snapper architecture

## Goals

1. One codebase, native feel on Windows, macOS and Linux (X11 and Wayland).
2. Capture to clipboard in under 150 ms after the hotkey on a typical laptop.
3. No network access unless the user configures an upload target.
4. Everything the GUI can do, the CLI can do.

## Process model

Snapper runs as a single process, `snapper-app`, that lives in the tray. The tray owns:

- `HotkeyService` (crate `global-hotkey`) which maps key chords to `Action`s.
- `CaptureService` (crate `snapper-capture`) which returns a `snapper_core::Image`.
- `EditorWindow` (egui via `eframe`) which opens on demand with a `snapper_editor::Document`.
- `OutputPipeline` which runs the configured steps: redact, copy, save, upload, notify.

`snapper-cli` links the same crates without the GUI so it can run headless in scripts.

## Data flow

```
hotkey -> Action::CaptureRegion
  -> overlay (frozen full-desktop screenshot, user drags a rectangle)
  -> CaptureService::crop(rect) -> Image
  -> Editor (optional, per settings)
  -> OutputPipeline [redact? -> clipboard -> save -> upload -> notify]
```

## Capture backends

| Platform | Backend | Notes |
|---|---|---|
| Windows | `xcap` (DXGI / GDI) | Per-monitor DPI aware manifest required |
| macOS | `xcap` (CoreGraphics). Recording reuses that capturer; ScreenCaptureKit is not linked | Needs Screen Recording permission |
| Linux X11 | `xcap` (XCB) | |
| Linux Wayland | `ashpd` XDG Desktop Portal Screenshot. ScreenCast recording is not implemented | User consent dialog per capture on some compositors. Scrolling capture is refused |

All coordinates inside Snapper are physical pixels on the virtual desktop. Each `Image` carries its `scale_factor` so the editor can show it at logical size.

## Editor

The document model in `snapper-editor` is a base `Image` plus an ordered `Vec<Layer>` of `Shape`s. Rendering for export uses `tiny-skia` so output is identical on every OS and testable with golden images. The egui editor draws a preview using the same shapes. Redaction shapes (blur, pixelate) are destructive on export: the pixels under them are replaced, never just covered.

## Settings

`~/.config/snapper/config.toml` (or the OS equivalent via the `directories` crate), parsed into `snapper_core::Settings` with `serde`. Unknown keys are kept and warned about, never dropped.

## Plugins

Upload targets implement `snapper_upload::UploadTarget`. Built-in targets are compiled in. Third-party targets in M3 run as external processes speaking JSON over stdio, so they cannot crash or compromise the app.

## Security

- No telemetry.
- Secrets for upload targets go in the OS keychain via the `keyring` crate, never in `config.toml`.
- Auto-redaction runs locally using OCR plus regex and entropy detectors.
