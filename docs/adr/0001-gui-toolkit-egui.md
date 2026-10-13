# ADR 0001: Use egui (eframe) for the GUI, not Tauri 2

Status: Accepted

## Context

Snapper needs a tray app, a full-screen selection overlay on every monitor, and a pixel-accurate annotation editor. Candidates were Tauri 2 (web UI in a system webview) and egui via eframe (immediate-mode, GPU rendered, pure Rust).

## Decision

Use egui and eframe (wgpu backend, glow fallback).

## Reasons

- The overlay and editor are pixel work. In egui the image, the selection and shapes are drawn in one Rust render loop with no IPC hop. In Tauri every mouse move would cross the JS bridge or need a canvas reimplementation in TypeScript.
- Single static binary with no WebView2 (Windows) or WebKitGTK (Linux) runtime dependency. WebKitGTK versions vary a lot across Linux distros.
- Startup time matters for a hotkey tool. eframe windows open in tens of milliseconds.
- The whole team writes Rust only. No second language or bundler.

## Consequences

- egui looks less native than a web UI. We accept that and invest in a clean theme.
- Accessibility in egui is improving through AccessKit, which we enable from day one.
- Settings screens are more work than HTML forms but are manageable.
