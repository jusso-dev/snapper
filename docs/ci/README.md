CI is kept here rather than in `.github/workflows` for now. To enable it, copy `ci.yml` to `.github/workflows/ci.yml` with a token that has the `workflow` scope.

`release.yml` is the tagged-release recipe (MSI, NSIS, DMG, app, AppImage, deb, rpm). Copy it to `.github/workflows/release.yml` when you want GitHub to run it. Signing steps and secret names are in [../RELEASING.md](../RELEASING.md).
