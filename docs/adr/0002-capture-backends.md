# ADR 0002: Capture with xcap, plus the XDG portal on Wayland

Status: Accepted

## Decision

Use `xcap` for screenshots on Windows, macOS and X11. Use `ashpd` (XDG Desktop Portal) on Wayland for screenshots and screencast. Use `scap` or ScreenCaptureKit directly for recording on macOS when `xcap` recording is insufficient.

## Reasons

`xcap` covers monitors and windows on all three platforms with one API. Wayland forbids direct screen reading, so the portal is the only legitimate path. Everything sits behind the `snapper_capture::Capturer` trait so backends can be swapped.

## Consequences

On Wayland some features (window list, frozen overlay) depend on the compositor. We degrade gracefully and document it.
