# Command line

`snapper-cli` is the scripting binary. `snapper-app` is the tray app. `edit` launches `snapper-app --open-editor`.

```sh
snapper-cli capture full --out shot.png
snapper-cli capture monitor 1 --format jpeg --delay 2 --clipboard
snapper-cli capture window 42 --out window.png
snapper-cli capture region 0,0,800,600 --out region.png
snapper-cli list monitors --json
snapper-cli list windows
snapper-cli edit shot.png
snapper-cli ocr shot.png
snapper-cli ocr 0,0,800,600
snapper-cli record --region 0,0,640,480 --duration 3 --out clip.gif
snapper-cli record --region 0,0,640,480 --duration 3 --out clip.mp4 --fps 30
```

`--json` prints a JSON object or array on stdout. `--verbose` raises the log level.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage, including a missing image path |
| 3 | Screen recording permission denied |
| 4 | Capture, OCR, or recording failed |

Without `--out`, a capture is saved with the directory and filename template from settings. `--clipboard` copies the image as well as saving it. The delay is in seconds.

`record` writes a GIF unless the path ends in `.mp4`. GIF frames are encoded with gifski. MP4 is piped to `ffmpeg`. If `ffmpeg` is missing, the command fails and tells you to install it or record a GIF.

`ocr` prints the recognised text. On Linux, build with `--features tesseract` and install `tesseract-ocr`. See `docs/platforms/linux-x11.md`.
