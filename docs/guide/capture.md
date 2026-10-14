# Capture

Snapper can grab the full screen, one monitor, one window, or a rectangle. The tray menu and the hotkeys start the same actions.

| Action | Default hotkey |
|---|---|
| Region | Print Screen |
| Window under the pointer | Alt+Print Screen |
| Full screen | Ctrl+Print Screen |
| Last region | Shift+Print Screen |
| Record | Ctrl+Shift+R |
| OCR | Ctrl+Shift+O |
| Scrolling capture | Ctrl+Shift+S |

Change them in Settings. A chord another action already uses is rejected.

The region overlay freezes the desktop. Drag a rectangle. Click a window to capture that window. Escape cancels.

## Scrolling capture

Choose Scrolling capture, drag the area that should scroll, and leave the pointer over the page. Snapper takes a frame, sends a wheel event, waits (250 milliseconds by default), and stitches the frames. It stops when the new frame matches the last one, when the image reaches the maximum height (30000 pixels by default), or when you press Stop.

The Stop button sits in a small window so you can end the capture early.

This does not run on Wayland. See `docs/platforms/linux-wayland.md`.

## After the capture

Settings decide what happens next: copy to the clipboard, save with the filename template, open the editor, or redact secrets before the image is copied or saved. Redaction is off until you enable it. A failed upload keeps the file that was already saved.
