# Linux (X11)

Capture uses `xcap` on X11. Set `SNAPPER_FORCE_X11=1` when you are on XWayland and want this backend instead of the portal.

The clipboard on Linux is owned by the process that wrote it. Snapper keeps the image available for 60 seconds, or until the next copy.

OCR uses Tesseract when you build with the feature and the library is installed:

```sh
sudo apt install tesseract-ocr
cargo build -p snapper-ocr --features tesseract
```

Without that feature, OCR returns a message that names `tesseract-ocr` and `--features tesseract`. The default workspace build leaves the feature off so other platforms do not need the library.

MP4 recording needs `ffmpeg`. GIF uses gifski inside the process.

Global hotkeys work through X11. See `docs/platforms/linux-wayland.md` for the Wayland limits.
