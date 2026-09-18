---
type: ChangeRequest
kind: feature
title: Preview window
description: Add --preview, a window showing the frames the model sees, built behind the preview cargo feature.
state: done
priority: medium
tags: [dev, preview, trust]
owner: felix
verified: { by: agent, at: 2026-09-18T23:57:16+02:00 }
---

# Problem

Without `--preview`, the server drives the desktop without showing anything. The
human operator cannot tell what the model is seeing or whether it is about to
act on the wrong window, which makes the server hard to trust and impossible to
supervise. The capture backend already produces RGB frames, so the missing piece
is purely presentation.

# Proposal

Implement the `--preview` window using `eframe`/`egui`, gated at compile time by
the `preview` cargo feature and at runtime by the flag. The window displays the
most recent frame from the capture backend, scaled to fit and refreshed as new
frames arrive, and it is display-only: it never takes input from the user and
never interferes with the session.

Because `eframe` requires the main thread, this change wires the entry point so
that the GUI event loop runs on the main thread while tokio and the MCP server
run on a worker thread. Without the flag, the main thread drives tokio as before.

# Decisions

- **`eframe`/`egui`.** Quickest route to a correct image view, and itself
  portable, which matches the extensibility goal.
  See [requirements.md](../../requirements.md).
- **Compile-time feature plus runtime flag.** The feature keeps GUI dependencies
  out of headless builds; the flag lets a build that has the feature still run
  without a window.
- **Display-only window.** The preview is a trust mechanism, not a second control
  surface. Adding input to it would create a second, confusing path to the same
  desktop.
- **The mirror effect is documented, not engineered away.** Wayland offers no
  reliable way to exclude one window from a portal capture, so the preview
  appears in what the model sees. The documentation recommends placing the
  preview on an output other than the captured one; the limitation is recorded
  as R11 in [requirements.md](../../requirements.md).
- **Sequenced third, ahead of the tools.** It needs only the capture backend, and
  having a live view early makes the rest of the milestone observable while the
  tool work lands.

# Acceptance criteria

- `desktop-use --preview` opens a window showing the captured frames, and the
  window updates as the desktop changes.
- The MCP server remains fully functional while the window is open: an MCP
  client can capture and act throughout.
- Closing the preview window does not crash the server or end the portal
  session; the remaining behaviour matches running without `--preview`.
- Without `--preview`, no window is created and no GUI event loop runs.
- With `--no-default-features`, the binary builds and `--preview` reports that
  the feature was not compiled in.
- The mirror effect and the recommended placement are documented for operators.

# Documentation

Updates the `README.md` added by the scaffold with the `--preview` flag, the
feature flag that controls it, and the mirror-effect limitation.

# Implementation plan

## Approach

One new module, `preview.rs`, compiled only when the `preview` feature is on,
holding both the window and the pump that feeds it.

**Getting frames to the window.** The capture backend already lives behind
`Arc<Backends>`, so it can be shared. A pump task on the tokio worker thread calls
`capture_frame` in a loop and posts each frame over a `std::sync::mpsc` channel,
waking the UI with `egui::Context::request_repaint`, which is documented as
callable from another thread. The app drains the channel once per painted frame
and keeps only the newest image, so a producer faster than the display cannot
build up latency by showing stale frames late.

**One texture, not one per frame.** The `TextureHandle` is allocated on the first
frame with `Context::load_texture` and then updated in place with
`TextureHandle::set`. Allocating a texture per frame is the obvious way to leak
GPU memory here, so the reuse is the point rather than an optimisation.

**`capture_frame` returns `Arc<Frame>`.** The pump polls, and polling a
by-value `Frame` would copy the pixels on every poll — several megabytes each
time, for a frame the backend is already holding in an `Arc` internally. The
trait returns the shared buffer instead, so the pump, and later the screenshot
tool, hand the same allocation around.

**Threading.** With `--preview`, the GUI event loop owns the main thread and
tokio runs on a worker thread serving MCP — the arrangement this request was
sequenced around. After the window closes, the main thread joins the worker, so
the server keeps serving: closing the window must not end the session.

**Failures are shown, not retried.** If the pump gets an error it posts it to the
window, which displays it in place of an image, and stops. Retrying on a timer
would re-prompt for consent whenever the failure was a denied prompt, which is
worse than showing the error once and letting the operator restart.

## Steps

- [x] Add `eframe` as an optional dependency behind the `preview` feature.
- [x] Change `CaptureBackend::capture_frame` to return `Arc<Frame>`, and follow
  it through the portal backend and `examples/dump_frame.rs`.
- [x] Write `preview.rs`: the pump, the app, the newest-frame drain, and the
  texture reuse.
- [x] Restructure `main.rs` for the main-thread/worker-thread split.
- [x] Replace the obsolete `PreviewUnavailable` error with one naming the missing
  feature.
- [x] Gate the `--preview` integration test on the feature, and add unit tests
  for the newest-frame drain.
- [x] Update `README.md`: the flag, the feature, and the mirror-effect
  limitation.
- [x] Verify on the live session.

# Verification

Verified on a live GNOME 50 session on Fedora 44 (aarch64) on 2026-09-18.

**The window opens and shows frames.** `desktop-use --preview` starts the pump,
negotiates the stream, and receives frames:

```
INFO desktop_use: capture backend: portal+pipewire
DEBUG desktop_use::preview: the preview pump started
INFO desktop_use::backend::frame_source: stream negotiated: 2560x1600, Bgrx layout, in DataType::MemFd buffers, stride 10240
DEBUG desktop_use::preview: the preview is receiving frames (2560x1600)
```

**The MCP server works while the window is open.** Driving a real `initialize`,
`notifications/initialized` and `tools/list` at the running process gets both
replies (`initialize` answered, `tools/list` returning an empty list) while the
window is up.

**`wgpu` works here, but only via the newer Asahi driver.** The Apple Silicon GPU
in this machine is driven by Vulkan, and the first driver that loads fails:

```
WARN wgpu_hal::vulkan::instance: terminator_CreateInstance: Received return code -9
     from call to vkCreateInstance in ICD /usr/lib64/libvulkan_dzn.so. Skipping this driver.
```

`dzn` is skipped and wgpu falls back to **Honeykrisp** (`Apple M1 (G13G B1)`,
Mesa 26.1.7), which works. This is worth knowing because it is a runtime risk
rather than a build risk: `wgpu` compiles fine either way, and a machine where
neither driver loaded would fail only when the window is opened. `glow`
(OpenGL) is the fallback if that ever happens, and was the lighter option.

**A flaw found by testing, and fixed.** On the first run the window produced no
visible frames and there was nothing in the log to explain why, because the pump
reported failures only to the window — invisible when the window is the very
thing being tested. The pump now logs `tracing::error!` alongside the on-screen
message, and logs at debug when it starts and when it first receives frames. The
frames had in fact been arriving all along: the missing evidence was the bug.

# Acceptance criteria status

All six hold on GNOME 50.

| Criterion | Status |
| --- | --- |
| `--preview` opens a window showing captured frames, updating as the desktop changes | holds |
| The MCP server stays fully functional while the window is open | holds |
| Closing the window does not stop the server or end the session | holds |
| Without `--preview`, no window and no GUI event loop | holds |
| `--no-default-features` builds and refuses `--preview` with a clear message | holds |
| The mirror effect and recommended placement are documented | holds |

**Closing the window** was verified by observation rather than by construction. A
client held the connection open, the window was closed by hand, and a request sent
afterwards was still answered:

```
21:56:22 INFO desktop_use::backend::frame_source: stream negotiated: 2560x1600, Bgrx layout, in DataType::MemFd buffers, stride 10240
21:56:55 INFO desktop_use: the preview window closed; the session is still open and the server keeps running
{"jsonrpc":"2.0","id":3,"result":{"tools":[]}}
```

The session and the server both outlive the window, which is what makes the window
a view of the session rather than its owner. `main` logs that line at the point
where the GUI loop returns and the worker is joined, so the property stays
observable rather than only argued for.
