---
type: Requirements
title: Requirements
description: The users, use cases, technology choices, and numbered requirements that define the first milestone.
tags: [wayland, mcp, requirements]
status: draft
---

# Requirements

## Users

### Agent developer (primary)

Builds autonomous or semi-autonomous agents that operate a Linux desktop: QA
sweeps across a GUI, filling in forms, GUI regression checks, or one-off "open
this application and do X" tasks. Technically strong — writes MCP client code —
but expects the server to absorb the Wayland protocol complexity rather than
push it into the agent.

Cares about: tool granularity it can reason about; deterministic, actionable
errors; screenshots cheap enough to call in a loop; and coverage across the
compositors its users actually run (GNOME, KDE, wlroots).

### Human operator (secondary)

A developer who runs `computer-use` on their own workstation, either to let an
assistant drive the machine or to watch and debug what an agent is doing.
Technically strong, but not necessarily fluent in Wayland internals.

Cares about: never being driven invisibly; predictable cost; and setup that
fails loudly and specifically rather than mysteriously.

## Use cases

Each use case is written so that an acceptance criterion can be derived from it.

- **UC1 — Observe the desktop** (agent developer): given a running server on a
  Wayland session, when the agent calls `screenshot`, then it receives an image
  of the desktop plus metadata (origin, size, scale factor, pointer position)
  good enough to locate UI elements. Success: text in the UI is legible in the
  returned image, and the metadata coordinates can be passed back to the input
  tools unchanged.
- **UC2 — Click a target** (agent developer): given a coordinate identified from
  a screenshot, when the agent calls `mouse_move` followed by `left_click`,
  then the pointer moves and the click is delivered to the application under it.
  Success: a following screenshot shows the click's effect.
- **UC3 — Type text** (agent developer): given a focused text field, when the
  agent calls `type_text` with a UTF-8 string, then the field contains that text.
- **UC4 — Send a shortcut** (agent developer): given an application with
  keyboard shortcuts, when the agent calls `key` with a key and modifiers,
  then the application handles the shortcut.
- **UC5 — Scroll a view** (agent developer): given a scrollable view, when the
  agent calls `scroll`, then the view scrolls by the requested amount.
- **UC6 — Consent once** (agent developer): given the server has never run, when
  the agent first captures the screen, then the user is prompted for consent
  exactly once, and later runs reuse that consent without prompting.
- **UC7 — Watch the agent** (human operator): given the operator starts the
  server with `--preview`, when the agent captures and acts, then a window shows
  the operator the same frames the model sees, so the session is never invisible.
- **UC8 — Fail usefully** (human operator): given the portal, a portal backend,
  or RemoteDesktop support is missing, when the server starts or the agent calls
  an input tool, then the error names the missing component and what to install
  or which compositor capability is absent.
- **UC9 — Scale and layout** (agent developer): given a HiDPI scale factor or a
  multi-monitor layout, when the agent captures and then acts, then coordinates
  read from the screenshot land where the agent intends.

## Technology choices

| Choice | Chosen | Why, and the alternative considered |
| --- | --- | --- |
| Language | Rust, edition 2024, `cargo` | Given in the project brief. |
| MCP SDK | `rmcp`, the official Rust SDK, with its `#[tool]` macros | Tracks the protocol and its schema, including tool-schema generation and framing. A hand-rolled JSON-RPC server would mean owning all of that for no gain. |
| MCP transport | stdio, behind a transport trait | Agents launch servers over stdio. A pluggable trait keeps HTTP/SSE open without committing to it now. |
| Screen capture | XDG Desktop Portal `ScreenCast` + PipeWire | The only compositor-agnostic route (GNOME, KDE, wlroots), and it works from inside a Flatpak sandbox. `wlr-screencopy` / `ext-image-copy-capture` is simpler and lower-latency but wlroots-only; X11 `XGetImage` and per-compositor D-Bus APIs (GNOME Shell, KDE) do not generalise. |
| Portal client | `ashpd`, which wraps `zbus` | The typed binding for exactly these portal interfaces; raw `zbus` would mean restating the interfaces by hand. |
| Frame transport | `pipewire` + `libspa` | The compositor delivers frames as PipeWire buffers; there is no alternative client in Rust. |
| Input injection | Portal `RemoteDesktop` | Same portability argument as capture, and it can share the consent session. The known gap — `xdg-desktop-portal-wlr` implements `ScreenCast` but not `RemoteDesktop` — is why input sits behind its own trait. Alternatives: wlroots virtual-input protocols, and kernel `uinput` (`ydotool`-style). |
| Async and threading | `tokio`; PipeWire's main loop on a dedicated thread, bridged by channels | PipeWire's loop does not integrate cleanly with tokio; forcing the integration produces subtle scheduling bugs. |
| Preview GUI | `eframe`/`egui`, behind a default-on `preview` cargo feature | Quickest route to a correct image view, and itself portable, which matches the extensibility goal. The feature flag drops the GUI dependency tree from headless builds. Alternatives: `pixels` + `winit` (leaner, less ergonomic), GTK4 (heavy, less portable). |
| Image encoding | `image` crate; PNG by default, JPEG opt-in; `max_dimension` default 1568 px | An agent reading text off a UI needs lossless by default, but an uncapped 4K screenshot is expensive in base64 and tokens, so downscaling and an opt-in lossy format are both required. 1568 px matches the common vision-model long-edge limit. |
| Errors and logging | `thiserror` for typed errors surfaced as MCP tool errors; `tracing` to stderr | Typed errors produce the actionable messages UC8 needs. Logging must never touch stdout, which is the MCP channel. |
| Licensing | MIT OR Apache-2.0 | The Rust convention. |

## Requirements

Priorities use the backlog's high/medium/low scale. "Milestone" marks what the
first milestone delivers; [Deferred](#deferred) lists what is deliberately left
out.

### Functional

| ID | Requirement | Trace | Priority | Milestone |
| --- | --- | --- | --- | --- |
| R1 | Capture the desktop through the portal `ScreenCast` interface and receive frames over PipeWire. | UC1 | high | yes |
| R2 | Establish one portal session per process, persist the user's source choice with a `restore_token`, and reuse it on later runs, so consent is requested once. | UC6 | high | yes |
| R3 | Expose the observe and control operations as MCP tools over stdio: `screenshot` and `zoom` for observation; `get_display_info` and `cursor_position` for geometry; `mouse_move`, `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click`, `left_click_drag`, `left_mouse_down`, `left_mouse_up`, and `scroll` for the pointer; `type`, `key`, and `hold_key` for the keyboard; and `wait` and `wait_for_change` for timing. | UC1–UC5 | high | yes |
| R4 | Inject pointer and keyboard input through the portal `RemoteDesktop` interface: absolute pointer motion, pointer button presses and releases — so clicks, drags, and holds are all expressible — scroll, UTF-8 text, and key presses, holds, and combinations. Modifiers can be held around a click, drag, or scroll rather than only as part of a key combination. | UC2–UC5 | high | yes |
| R5 | Return captured frames as MCP image content — PNG by default, JPEG opt-in — and accept a `max_dimension`, so an agent can bound the cost of a screenshot it takes in a loop. | UC1 | high | yes |
| R6 | Normalize coordinates across the logical desktop, the captured stream's pixels, and the scale factor, so a coordinate read from `screenshot` or `get_display_info` is accepted unchanged by the input tools. | UC9 | high | yes |
| R7 | Provide a `--preview` window showing the frames the model sees, so the human operator can watch the session. | UC7 | medium | yes |
| R8 | Report display and capability information — monitors, logical size, scale factor, stream resolution, pointer position, and whether input injection is available for the current backend — through the `get_display_info` tool. | UC1, UC8 | medium | yes |

### Non-functional

| ID | Requirement | Trace | Priority | Milestone |
| --- | --- | --- | --- | --- |
| R9 | Put capture and input behind `CaptureBackend` and `InputBackend` traits, so another windowing system or platform can be added without changing the MCP tool implementations. | Project brief: extensibility | high | yes |
| R10 | Fail with actionable diagnostics: detect a missing session bus, missing portal, or missing backend at startup; report "RemoteDesktop is not supported by this portal backend" for input rather than a raw D-Bus error; and never write logs to stdout, which is the MCP channel. | UC8 | high | yes |
| R11 | Never drive the desktop invisibly: `--preview` is the visibility mechanism, and its limitation — the preview window is itself captured, producing a mirror effect — is documented rather than left as a surprise. | UC7 | medium | yes |
| R12 | Keep the round trip fast enough to call in a tool-calling loop and avoid unnecessary frame copies: an end-to-end `screenshot` on a single 1080p output completes well under a second, with an initial target of ≤250 ms median on GNOME or KDE and the measured figure recorded once the capture path exists. | UC1 | medium | yes |

### Constraints

| ID | Requirement | Trace | Priority | Milestone |
| --- | --- | --- | --- | --- |
| R13 | License the project as MIT OR Apache-2.0, and document the runtime prerequisites: a graphical session with `DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR`, and `WAYLAND_DISPLAY`, plus `xdg-desktop-portal` and a backend. | Brief, UC8 | low | yes |

## Initial milestone

**"An MCP client can see and act on a Wayland desktop."** An agent connects over
stdio, obtains a single consented portal session, captures the screen, and
injects pointer and keyboard input — with `--preview` letting the human operator
watch. It covers R1 through R13.

Two scope boundaries are deliberate:

- **On wlroots compositors (Sway, Hyprland, river), the first milestone is
  capture-only.** `xdg-desktop-portal-wlr` implements `ScreenCast` but not
  `RemoteDesktop`, so input injection has no portal path there. This is precisely
  why R9's `InputBackend` trait exists. GNOME and KDE support both.
- **DMA-BUF readback was the milestone's main technical risk, and it did not
  materialise.** Offering only pixel layouts without a `VideoModifier` keeps the
  negotiation on shared-memory (`memfd`) buffers, which GNOME honours, so capture
  runs entirely on the CPU with no GPU import or readback. The backend still
  detects a DMA-BUF buffer and fails with a diagnostic naming it, so a compositor
  that insists on DMA-BUF produces a clear error rather than a hang; should one
  turn up, importing those buffers becomes its own change request.

## Deferred

Listed so that later change requests can pick them up:

- Accessibility-tree introspection (AT-SPI), so an agent can see a structured
  widget tree instead of only pixels.
- Importing DMA-BUF frames, for a compositor that offers nothing else. See the
  [backlog request](dev/backlog/dma-buf-frame-import.md).
- Additional input backends: the wlroots virtual-input protocols
  (`wlr-virtual-pointer`, `virtual-keyboard`) and kernel `uinput`, either of
  which would make input work on wlroots.
- Additional capture backends: `wlr-screencopy` / `ext-image-copy-capture` as a
  faster wlroots path.
- Other platforms and windowing systems: X11, macOS, and Windows.
- An HTTP/SSE MCP transport.
- Multi-monitor and window-scoped capture, and capture of virtual outputs.
- Video or recording output, and OCR.
- Excluding the preview window from what is captured.
