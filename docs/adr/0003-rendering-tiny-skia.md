# ADR 0003: Render exported annotations with tiny-skia

Status: Accepted

## Decision

Exports are rendered on the CPU with `tiny-skia` and text with `cosmic-text`. The egui preview mirrors this.

## Reasons

Deterministic output on all platforms lets us test with golden PNGs in CI. GPU output varies by driver.
