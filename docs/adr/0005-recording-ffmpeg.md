# ADR 0005: Encode MP4 with an external ffmpeg, GIF in-process

Status: Accepted

## Decision

GIF encoding uses the `gifski` crate in-process. MP4 uses a system `ffmpeg` if found, with an optional download prompt. We do not statically link ffmpeg.

## Reasons

Statically linking ffmpeg brings LGPL and patent complexity and a huge binary. gifski gives high-quality GIFs in pure Rust.
