# macOS

Snapper is pre-alpha and does not yet provide a supported screen-capture workflow. When capture support is implemented, macOS may ask for Screen Recording permission. Grant permission in **System Settings → Privacy & Security → Screen Recording**, then quit and reopen Snapper if the system asks you to restart the app.

For development, reset Screen Recording permission for the Snapper bundle identifier with:

```sh
tccutil reset ScreenCapture au.jusso-dev.snapper
```

This removes that app's saved authorization so macOS can ask again. Apple documents `tccutil reset` as a way to reset protected-resource access during development. See [Apple's permission-reset guidance](https://developer.apple.com/documentation/xcode/resetting-access-to-protected-resources-in-macos).
