# Windows

Capture uses `xcap`. Windows may show a prompt the first time a program records the screen.

OCR uses `Windows.Media.Ocr` through the `windows` crate (`Media_Ocr` and `Graphics_Imaging`). The engine is created with `OcrEngine::TryCreateFromUserProfileLanguages`. That code is compiled only on Windows. It was not type-checked on the macOS machine used for this tree. If the system OCR language is missing, the call fails with the backend error.

GIF recording does not need an extra install. MP4 needs `ffmpeg` on `PATH`. If it is missing, Snapper says so and a GIF is still available.

Global hotkeys use `global-hotkey`. A chord the operating system refuses is reported as a conflict.

Installers are MSI and NSIS, built with `cargo packager`. Signing is described in `docs/RELEASING.md`.
