# ADR 0004: Upload targets as a trait now, external processes later

Status: Accepted

## Decision

M2 ships built-in targets (S3, R2, Imgur, custom HTTP) implementing `UploadTarget`. M3 adds external plugins: an executable named `snapper-upload-<id>` that receives a JSON request on stdin and returns JSON on stdout.

## Reasons

Built-ins keep v1 simple. Process plugins avoid an unstable Rust ABI and isolate third-party code.
