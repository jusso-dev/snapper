# macOS

Snapper captures the screen with `xcap`. The first capture asks for Screen Recording permission. The usage string in the app bundle is `NSScreenCaptureUsageDescription` in `crates/snapper-app/macos/Info.plist`.

A `cargo run` binary is not an app bundle, so macOS may show the binary name in System Settings rather than Snapper, and the Dock icon stays visible. `LSUIElement` hides the Dock icon only after the app is packaged.

## Reset Screen Recording permission

```sh
tccutil reset ScreenCapture au.jusso-dev.snapper
```

Quit Snapper first. The next capture asks again. The bundle identifier is `au.jusso-dev.snapper`.

OCR uses the Vision framework (`VNRecognizeTextRequest`, accurate level). It stays on the machine.

Recording captures frames through the same screenshot backend. ScreenCaptureKit is not linked. MP4 needs `ffmpeg` on `PATH`. GIF does not.

Scrolling capture sends a mouse wheel event with `enigo` after each frame. Grant Accessibility permission if macOS blocks synthetic input.

## Retina displays

macOS reports display and window bounds in points. Snapper converts them to physical pixels, so a Retina display captures at full resolution. With displays of different scales, each display's position is multiplied by the largest scale and its size by its own scale. Displays never overlap in that layout, and a full-desktop capture shows black gaps beside the lower-scale displays.
