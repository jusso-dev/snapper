# Editor

The editor opens when you choose it after a capture, from `snapper-cli edit <path>`, or from a workflow step named `annotate`.

Tools are listed in `docs/accessibility.md`, including the keys. Select is `S`. Save stays `Ctrl+S`.

Automatic redaction adds pixelate blocks and marks them for review. Those blocks have a dashed outline and a banner that says "Review redactions". Remove automatic redactions deletes every one of them. You can also select a single block and press Delete.

Copy, Save, and Upload are on the bottom bar. Upload uses the first configured target, saves the file, copies the plain URL, and notifies you. If the upload fails, the file remains.

Crop limits what is flattened. Undo and redo cover annotations and the crop.
