# Validation that was not run

Automated tests cover the logic that does not need a display server, a paid certificate, or a third-party account. The checks below are still outstanding. Do not treat them as passed.

## Capture, OCR, and recording

- `snapper-cli capture full --out /tmp/x.png` on a machine with a display, then open the PNG.
- `snapper-cli list monitors --json` on Windows, macOS, and Linux.
- OCR of a screenshot of this repository's README, on each operating system, returns the heading text. On this Mac the Vision engine is compiled in, but that README screenshot was not recognised as part of this change.
- A 10 second 800×600 GIF on a mid-range laptop, with no dropped frames, and memory under 500 MB for a 60 second recording. The queue is bounded at 60 frames in a unit test. The live recording was not measured.
- An MP4 opened in a common player. The encoder test is skipped when `ffmpeg` is missing, and a produced file was not played back.
- Scrolling a long web page and checking the stitch for duplicated or missing rows.
- A click ring during recording. The cursor dot is stamped from the pointer position. A global mouse-button hook is not connected, so the click ring is only covered by `stamp_cursor` in unit tests.

## Uploads and accounts

- A real upload to Cloudflare R2 and to AWS S3 that returns a working public URL. The ignored MinIO test is `cargo test -p snapper-upload -- --ignored minio_put_returns_a_url` with `MINIO_ENDPOINT`, `MINIO_ACCESS_KEY`, `MINIO_SECRET_KEY`, and `MINIO_BUCKET`. It was not run.
- Imgur with a real Client-ID. Tests use a local fake server.

## Packaging and signing

- Each installer (MSI, NSIS, DMG, `.app`, AppImage, deb, rpm) installs and launches Snapper on a clean virtual machine, and uninstall removes the binaries.
- `actionlint` on `docs/ci/release.yml` after it is copied to `.github/workflows`. It is not a live workflow in this tree.
- A signed Windows installer shows the publisher name.
- `spctl -a -vv` on a notarised macOS app.
- `cargo packager --release -p snapper-app` was not run.

## Accessibility

- NVDA on Windows, VoiceOver on macOS, and Orca on Linux announcing the editor toolbar buttons. AccessKit is enabled in the build. No screen reader was used. See `docs/accessibility.md`.

## Tesseract

- A Linux build with `--features tesseract` and the `tesseract-ocr` package installed. The default build leaves the feature off so macOS and Windows do not need the library. The Windows OCR module is behind `cfg(windows)` and was not type-checked on this Mac.
