# Workflows

A workflow is a name, a hotkey, a capture, and a list of steps. Edit it on the Workflows tab. One step per line:

```
redact
annotate
save
upload r2
copy_link
notify
```

`redact` runs OCR on the machine and pixelates emails, keys, tokens, card numbers, and the other detectors listed in the redaction tests. `annotate` opens the editor. Redactions made before that step are items you can remove. The remaining steps run when you save. `copy_link` writes the URL as plain text, Markdown (`![name](url)`), or an HTML image, using the link format on the workflow.

Capture can be a region, the window under the pointer, or the full screen.

An empty hotkey is left unregistered. The workflow still appears in the tray menu.

Example: a bug report captures a region, redacts, opens the editor, uploads to the target named `r2`, and copies a Markdown link.
