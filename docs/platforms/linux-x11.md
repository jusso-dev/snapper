# Linux with X11

Snapper is pre-alpha and does not yet provide a supported capture workflow. A future X11 capture backend will depend on an available X server and the user's X11 authorization. In a remote or containerized session, check that `DISPLAY` is set and that the process has access to the correct X authority file.

Do not copy another user's Xauthority cookie or weaken access control to troubleshoot a capture failure. Report the desktop environment, session type, and relevant error in an issue.
