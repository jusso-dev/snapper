# Accessibility

The desktop app is built with eframe's `accesskit` feature. Toolbar buttons, the review banner, and the settings controls use the widget text as their name, so a screen reader can read "Fit", "Copy", "Save", "Upload", "Remove automatic redactions", and the settings tabs.

## Keyboard

Editor tools, other than drawing on the canvas, have keys:

| Key | Tool or action |
|---|---|
| S | Select |
| A | Arrow |
| R | Rectangle |
| E | Ellipse |
| H | Highlight |
| T | Text |
| N | Step number |
| B | Blur |
| P | Pixelate |
| C | Crop |
| Ctrl+Z | Undo |
| Ctrl+Y | Redo |
| Ctrl+C | Copy |
| Ctrl+S | Save |
| Delete or Backspace | Delete the selection |
| Escape | Cancel the current gesture |

Drawing a shape still needs the pointer. Pan with the middle button. Zoom with Ctrl and the scroll wheel.

Workflows and the built-in capture actions also have global hotkeys, configured in Settings. An empty workflow hotkey is not registered, so it does not collide with another chord. Start that workflow from the tray menu instead.

## Screen readers

These runs have not been done:

- NVDA on Windows
- VoiceOver on macOS
- Orca on Linux

Until they are, do not claim the toolbar is announced. The result belongs in `docs/VALIDATION.md` as well.
