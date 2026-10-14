# Releasing Snapper

Local packaging uses [`cargo-packager`](https://github.com/crabnebula-dev/cargo-packager). The metadata lives on `snapper-app` in `crates/snapper-app/Cargo.toml`. Cargo itself ignores it. The CI recipe is `docs/ci/release.yml`. It is not a live workflow until someone copies it to `.github/workflows/release.yml`.

No certificate, notarisation key, or minisign secret is stored in this repository. The steps below name the secrets. You create the values.

## Installers

```sh
cargo install cargo-packager --locked
cargo packager --release -p snapper-app --formats app,dmg
```

On Windows use `--formats msi,nsis`. On Linux use `--formats appimage,deb,rpm`. Linux packaging needs `patchelf`. AppImage builds also want `librsvg2-bin` when the tool rasterises icons.

Icons are in `crates/snapper-app/icons/` (PNG from 16 to 1024 pixels, plus `icon.ico` and `icon.icns`). The macOS bundle merges `crates/snapper-app/macos/Info.plist`, which sets `LSUIElement` and `NSScreenCaptureUsageDescription`.

A clean virtual machine install, and an uninstall that removes the binaries, has not been run. See `docs/VALIDATION.md`.

## Windows signing

Use Azure Trusted Signing or another Authenticode certificate. The release job reads these secret names and does not contain the secret values:

- `WINDOWS_SIGN_TENANT_ID`
- `WINDOWS_SIGN_CLIENT_ID`
- `WINDOWS_SIGN_CLIENT_SECRET`
- `WINDOWS_SIGN_ACCOUNT`
- `WINDOWS_SIGN_PROFILE`

Sign the installer so Windows can show the publisher name. Timestamp the signature. A signed installer on a clean machine has not been checked here.

## macOS signing and notarisation

Hardened runtime uses `crates/snapper-app/entitlements.plist`. Screen recording is a system permission prompt, not an entitlement.

```sh
codesign --force --options runtime --timestamp \
  --entitlements crates/snapper-app/entitlements.plist \
  --sign "$APPLE_SIGNING_IDENTITY" \
  "Snapper.app"
xcrun notarytool submit Snapper.dmg --wait \
  --key "$APPLE_NOTARY_KEY" \
  --key-id "$APPLE_NOTARY_KEY_ID" \
  --issuer "$APPLE_NOTARY_ISSUER"
xcrun stapler staple Snapper.dmg
```

Secret names: `APPLE_SIGNING_IDENTITY`, `APPLE_NOTARY_KEY`, `APPLE_NOTARY_KEY_ID`, `APPLE_NOTARY_ISSUER`.

`spctl -a -vv Snapper.app` has not been run against a notarised build. See `docs/VALIDATION.md`.

## Update manifests

Snapper checks `latest.json` when `check_updates` is true (the default). It verifies the detached minisign signature and, if the version differs, shows a notification. It never downloads or installs the update.

Generate a key pair on a machine you trust. Do not commit the secret key.

```sh
minisign -G -p minisign.pub -s minisign.key
```

Put the public key string into `BUNDLED_PUBLIC_KEY` in `crates/snapper-app/src/update.rs`. The copy in the tree is a placeholder with no matching secret in the repository. Replace it before a release you want the app to trust.

Store the secret key as `MINISIGN_SECRET_KEY`. For each release:

```sh
minisign -S -s "$MINISIGN_SECRET_KEY" -m latest.json -x latest.json.minisig
```

`latest.json` looks like this:

```json
{"version":"0.1.0","notes":"What changed.","url":"https://github.com/jusso-dev/snapper/releases/tag/v0.1.0"}
```

Publish both files on the GitHub release. The default URL is `https://github.com/jusso-dev/snapper/releases/latest/download/latest.json`. `SNAPPER_UPDATE_URL` overrides it for a fake server.

A unit test signs a manifest with a throwaway key, accepts it, and rejects a tampered body. That test does not use a paid certificate.
