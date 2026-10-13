# Wayland capture

On a Wayland session Snapper takes screenshots through the XDG Desktop Portal (`org.freedesktop.portal.Screenshot`), using the `ashpd` crate. Direct X11 capture on Wayland usually returns a black image, so the portal is the path that compositors allow.

`default_capturer()` selects this backend when `WAYLAND_DISPLAY` is set and `SNAPPER_FORCE_X11` is not. Set `SNAPPER_FORCE_X11=1` to keep the `xcap` backend, for an XWayland session where that is what you want.

## What you need installed

- `xdg-desktop-portal`
- The backend for the compositor: `xdg-desktop-portal-gnome`, `xdg-desktop-portal-kde`, or `xdg-desktop-portal-wlr` (Sway, Hyprland, and other wlroots desktops)

## Modes

- Non-interactive (`capture_all`, `capture_monitor`): the portal is asked for a full screenshot with `interactive` set to false. Snapper crops a region from that image itself. The selection overlay does that crop.
- Interactive (`WaylandPortalCapturer::capture_interactive`): the compositor shows its own dialog so the user can pick a region or an output before the image comes back.

## Prompts

The portal, not Snapper, decides whether a dialog appears.

- GNOME often asks for screenshot permission the first time, then may stay quiet for non-interactive captures in the same session.
- KDE Plasma may show an access prompt the first time an application requests a screenshot.
- Sway and Hyprland, through `xdg-desktop-portal-wlr`, usually open a chooser (output or region) even when the request is non-interactive. That chooser is the compositor's, and dismissing it cancels the capture.

These prompts are the documented portal behaviour. This repository's Windows build machine cannot run GNOME, KDE, Sway, or Hyprland, so they have not been exercised here.

## Limits

- There is no portable window list on Wayland. `windows()` and `capture_window()` return "not supported on Wayland" instead of failing inside the backend.
- The portal does not report monitor layout or scale. Snapper exposes one synthetic monitor named `Wayland`, id `1`, origin `(0, 0)`, scale `1.0`, and the width and height of the screenshot. `monitors()` takes a screenshot to learn that size.
- The screenshot is a temporary file with a `file://` URI. Snapper loads it and deletes it, including when the file is not a readable image.
- Recording is not part of this backend.

## Global hotkeys

`global-hotkey` 0.6 registers shortcuts through X11. On a Wayland session that grab does not reach other applications, and Snapper does not call the XDG GlobalShortcuts portal (`org.freedesktop.portal.GlobalShortcuts`). That portal is the Wayland path for compositor-wide shortcuts and is out of scope here. Use the tray menu on Wayland. `SNAPPER_FORCE_X11=1` only switches capture back to `xcap`; it does not enable GlobalShortcuts.

A hotkey the compositor or another program already owns is reported as a conflict. The notification names the action and the chord.

## Try it

```sh
WAYLAND_DISPLAY=wayland-0 cargo run -p snapper-capture --example grab
WAYLAND_DISPLAY=wayland-0 cargo run -p snapper-capture --example grab -- --windows
```

The second command should print the "not supported on Wayland" message and exit 2.
