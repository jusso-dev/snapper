# Linux with Wayland

Snapper is pre-alpha and does not yet provide a supported capture workflow. Wayland desktop environments normally mediate screen capture through desktop portals so the user can choose what to share. Snapper's implementation and supported portal backends are still being developed; there is no Snapper-specific permission toggle to configure yet.

When capture support lands, check that an XDG Desktop Portal backend is installed for your desktop environment and that the portal service is running. The portal may show a screen or window chooser for each capture session. See the upstream [XDG Desktop Portal screenshot interface](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.Screenshot.html).

If capture fails, include your desktop environment, Wayland compositor, portal backend, and error text in the issue. Do not disable the portal or grant broader access as a workaround.
