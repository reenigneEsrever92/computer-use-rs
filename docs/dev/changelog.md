---
type: Changelog
title: Changelog
description: Shipped changes to desktop-use, newest first.
tags: [dev, changelog]
status: draft
---

# Changelog

Shipped changes, newest first.

## 2026-09-19

- **RemoteDesktop input backend** — adds the `mouse_move`, `mouse_click`,
  `mouse_scroll`, `type_text`, and `key_press` tools, injecting through the
  portal's `RemoteDesktop` interface on the session capture already negotiated;
  `mouse_move` converts logical coordinates into the stream's pixels, and a
  position the portal refuses is reported by name. See
  [RemoteDesktop input backend](backlog/input-backend-remote-desktop.md).
- **Screenshot and display-info tools** — the MCP tool list is no longer empty:
  `screenshot` returns the desktop as PNG or JPEG with the metadata needed to map
  image coordinates back to logical ones, and `get_display_info` reports the
  geometry and whether input can be injected. See
  [Screenshot and display-info tools](backlog/screenshot-display-info-tools.md).

## 2026-09-18

- **Preview window** — `--preview` opens a window showing the frames the model
  sees, behind the `preview` cargo feature and fed by a pump that keeps only the
  newest frame; closing the window leaves the session and the server running. See
  [Preview window](backlog/preview-window.md).
- **Portal and PipeWire capture backend** — capture negotiates one consented
  portal session, reads frames from it over PipeWire, and converts
  `BGRx`/`RGBx`/`BGRA`/`RGBA` buffers to packed RGB, asking the compositor for
  shared-memory buffers explicitly; `examples/dump_frame.rs` captures a frame to a
  PNG. See
  [Portal and PipeWire capture backend](backlog/portal-pipewire-capture-backend.md).
- **Scaffold the MCP server** — the `desktop-use` binary now serves MCP over
  stdio with an empty tool list, refuses to start outside a graphical session
  while naming the missing variable, and defines the `CaptureBackend` and
  `InputBackend` traits. See
  [Scaffold the desktop-use server](backlog/scaffold-desktop-use.md).
