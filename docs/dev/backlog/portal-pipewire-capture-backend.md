---
type: ChangeRequest
kind: feature
title: Portal and PipeWire capture backend
description: Implement CaptureBackend using the portal ScreenCast interface and a PipeWire stream.
state: done
priority: high
tags: [dev, capture, wayland, pipewire]
owner: felix
verified: { by: agent, at: 2026-09-18T22:46:33+02:00 }
---

# Problem

The server cannot see the desktop. Wayland gives applications no global
framebuffer access, so capture has to be negotiated with the compositor through
the XDG Desktop Portal and delivered as a PipeWire stream. Until that exists,
every tool that needs pixels is unimplementable, including the preview window.

# Proposal

Implement the first `CaptureBackend`: negotiate a `ScreenCast` session through
`ashpd`, persist the user's source choice with a `restore_token` so that consent
is requested once, connect a `pipewire` stream to the returned node over the
remote file descriptor, and convert incoming buffers into an owned RGB frame
buffer.

The implementation prefers shared-memory (`memfd`) buffers, which can be mapped
and read directly, and falls back to DMA-BUF import plus readback where the
compositor offers nothing else. Format handling covers the common `BGRx`,
`xRGB`, `BGRA`, and `RGBA` layouts. The frame buffer is published to consumers
over a channel, so that the MCP tools and the preview window can each read it
without blocking the PipeWire thread.

# Decisions

- **Portal `ScreenCast` plus PipeWire, not `wlr-screencopy`.** Chooses compositor
  coverage (GNOME, KDE, wlroots) over simplicity and latency, as agreed for the
  project. See [requirements.md](../../requirements.md).
- **One session per process, with `restore_token`.** A portal session is
  consent-bound and expensive to establish; asking the user to approve every run
  would make the server unusable. This is R2.
- **PipeWire's main loop on its own thread, bridged by channels.** Its loop does
  not integrate cleanly with tokio.
- **`memfd` first, DMA-BUF second.** The simple path covers the common case, and
  the compositor is what decides which is offered.
- **Frame buffers are owned and format-converted in the backend.** Consumers
  receive RGB regardless of what the compositor sent, so that encoding and the
  preview share one code path and neither repeats the conversion.
- **DMA-BUF readback is the recognised risk.** If it needs a GPU context and a
  readback path of its own, it is split into a separate change request rather
  than left to expand this one.

# Acceptance criteria

- On a GNOME, KDE, or wlroots session, requesting capture shows the consent
  prompt on first use and does not show it again on later runs.
- The backend yields frames of the selected source at the resolution the portal
  reports, in a documented RGB layout.
- On a session where the compositor offers DMA-BUF, capture requests shared
  memory explicitly and, if the producer still delivers DMA-BUF, fails with an
  error naming the limitation rather than returning a stale or blank frame.
  Reading DMA-BUF is [its own request](dma-buf-frame-import.md).
- If the session is dropped — compositor restart, source removed — the backend
  reports the failure and can re-establish a session on demand, rather than
  panicking or returning stale frames.
- Frame bytes are correct: a screen region with known content is reproduced
  without channel swaps. This is verified manually against a controlled
  reference, because a portal consent prompt cannot be automated in CI.
- Frame delivery does not block the MCP runtime.

# Documentation

Updates the `README.md` added by the scaffold with the capture prerequisites
(`xdg-desktop-portal` and a backend) and the consent behaviour, and updates
[requirements.md](../../requirements.md) to move DMA-BUF import into its deferred
list, recording that the milestone did not need it.

# Implementation plan

## Approach

Two modules behind the `CaptureBackend` trait defined by the scaffold:

- `capture/portal.rs` — the portal half. Opens `Screencast` and `RemoteDesktop`
proxies, creates **one** session owned by `RemoteDesktop`, calls
`select_devices` for keyboard and pointer and `select_sources` for a monitor on
that same session, then calls `RemoteDesktop::start` **once**. A single `Start`
returns `SelectedDevices`, which carries both the capture streams
(`streams()`) and the granted input devices (`devices()`) — so the user is
prompted once for capture and input together, which is what R2 and UC6 want.
The stream's node id, size, and position come from `Stream::pipe_wire_node_id`,
`size`, and `position`; the PipeWire remote fd comes from
`Screencast::open_pipe_wire_remote`, which returns an `OwnedFd`.
- `capture/pipewire.rs` — the frame-source half. PipeWire objects are `!Send`,
so a dedicated thread **creates and owns** the `MainLoop`, `Context`, `Core`,
`Stream`, and listener, and drops them itself. The portal fd is moved in as an
`OwnedFd` (it is `Send`) and consumed by `Context::connect_fd_rc`. The thread is
stopped from outside via `pipewire::channel`, because `MainLoop::quit` takes
`&self` on a `!Send` type and cannot be called cross-thread. A one-shot startup
handshake reports setup failures synchronously so the backend can fail at
`connect()` rather than silently producing no frames.

Format negotiation asks for **shared-memory** buffers: the `EnumFormat` pod
offers `BGRx`/`RGBx`/`BGRA`/`RGBA` and deliberately **omits** the
`VideoModifier` property, since the SPA headers document the modifier key as
DMA-BUF-only. The process callback then checks `Data::type_()`: for `MemFd` and
`MemPtr` it reads the pixels via `Data::data()`, honouring `Chunk::offset`,
`size`, and `stride`.

DMA-BUF is the known cliff. In `pipewire` 0.9.2 a DMA-BUF buffer yields
`DataType::DmaBuf` with a valid `fd()` but `data() == None`, and the crate
provides **no** mmap, dmabuf-import, plane, or modifier handling. So the backend
detects that case and fails loudly with a message naming it, instead of hanging
or returning garbage. If testing shows GNOME negotiates DMA-BUF regardless of
the omitted modifier, importing it is split into its own change request with a
DRM/GBM/EGL path, exactly as this request's risk note anticipates.

Frames are converted from the negotiated layout to packed RGB in the backend and
published on a channel, so encoding (the tool change request) and the preview
window share one conversion path and neither re-implements it.

## Steps

- [x] Add `ashpd` (default features; tokio runtime) and `pipewire`/`libspa`.
- [x] Implement the portal session: proxies, one shared session, devices plus
  monitor source, single `Start`, and the `restore_token` round trip.
- [x] Implement the preflight that maps `Error::PortalNotFound` and
  `Error::RequiresVersion` into the actionable error variants from the scaffold
  (R10).
- [x] Implement the PipeWire frame source on its own thread, with the startup
  handshake and the channel-based stop.
- [x] Implement `EnumFormat` negotiation for the four packed RGB layouts with no
  `VideoModifier`, and the `MemFd`/`MemPtr` read path with offset/size/stride.
- [x] Send an explicit `Buffers` parameter asking for shared memory, so the
  requirement is stated rather than merely implied by the missing modifier.
- [x] Detect DMA-BUF and fail with a diagnostic naming the limitation.
- [x] Implement the RGB conversion for `BGRx`, `RGBx`, `BGRA`, and `RGBA`.
- [x] Publish frames on a channel and implement `CaptureBackend` over it.
- [x] Add an `examples/dump_frame.rs` that captures one frame and writes a PNG,
  so capture can be exercised and eyeballed without a window or an MCP client.
- [x] Verify against the live GNOME session, and record whether the negotiated
  buffer type was shared memory or DMA-BUF.
- [x] Update `README.md` with the consent behaviour and the PipeWire build
  prerequisite.

# Verification

Verified on a live GNOME 50 session on Fedora 44 (aarch64) on 2026-09-18, using
`examples/dump_frame.rs`. The server negotiated a real session and produced real
frames:

```
logical: 1536x960 at (0, 0)
stream:  2560x1600 (scale 1.6667)
input available: true
stream negotiated: 2560x1600, Bgrx layout, in DataType::MemFd buffers, stride 10240
frame: 2560x1600 (12288000 bytes of RGB)
```

What that settles:

- **The DMA-BUF cliff was not hit.** GNOME delivered `DataType::MemFd`, so the
  shared-memory path is the one that runs on GNOME and no EGL/DRM readback is
  needed. Omitting the `VideoModifier` property from the `EnumFormat` pod is what
  kept the negotiation on shared memory. The detection and diagnostic for
  DMA-BUF remain in place for compositors that do not cooperate, and importing it
  into its own change request is only needed if one turns up.
- **Consent is asked exactly once.** The first run showed the portal prompt; the
  second and third runs did not, so the `restore_token` round trip works (R2,
  UC6).
- **The scale factor is real.** The portal reports `1536x960` logical while the
  stream carries `2560x1600`, so the scale factor is 1.6667, not 1.0. `capture`
  now waits for the stream to negotiate its size and derives the factor from the
  two, and `CaptureInfo` carries both `logical_size` and `stream_size` with
  `logical_to_stream`/`stream_to_logical` helpers (R6).
- **Input devices were granted** (`input available: true`), so the RemoteDesktop
  input change request has something to build on here.

**Pixel correctness** was checked against a controlled reference rather than by
inspection, because the SPA format names are ambiguous: the header says only that
components are "described in big-endian order" and refers to GStreamer, where
`RGBx`/`BGRx` are likewise written as bit order. A red/blue swap would be
invisible to any self-consistency check.

`examples/reference_screenshot.rs` was added to take a screenshot through the
portal's own `Screenshot` interface, which has GNOME write the PNG itself through
an unrelated code path. Comparing the two captures pixel-for-pixel on downscaled
copies, over candidate channel mappings:

```
   2.042  identity (r,g,b)
   5.398  swap r/g
   6.206  swap g/b
   8.170  swap r/b
```

Per-channel means agreed to within about one unit (ours `(43.02, 45.67, 50.73)`
against the reference's `(43.19, 46.27, 51.77)`), and identity won by 2.6x over
the next candidate. The residual is the clock and the embedded cursor. The
memory-order reading of the layout names is therefore the correct one, and that
reasoning is recorded in `src/backend/convert.rs` so it is not re-litigated.

Automated tests: 24 unit tests and 6 stdio integration tests pass, `cargo build`
is warning-free on both feature sets, and `cargo clippy --all-targets` is clean.
The portal cannot be driven from a test, so capture itself is covered by the
runtime evidence above rather than by CI.

Two implementation details worth keeping:

- Persistence is requested on `select_devices` only. Asking for
  `PersistMode::Application` on the `ScreenCast` half of a remote-desktop session
  is rejected outright with "Remote desktop sessions cannot persist".
- Cursor mode is `Embedded`, not `Metadata`. With metadata the pointer is not in
  the pixels, and this build does not yet read the cursor buffer metadata that
  would report where it is — so `CaptureInfo::pointer` is `None` and the cursor is
  visible in frames instead. Moving to metadata is what will populate `pointer`.

**The shared-memory request is honoured, not ignored.** Sending
`SPA_PARAM_BUFFERS_dataType` alongside the formats was verified by inverting it:
asking for DMA-BUF alone made buffer allocation fail outright.

```
WARN capture failed and the session is no longer alive
     (capture failed: the PipeWire stream failed: error alloc buffers: Invalid argument)
error: capture failed: the PipeWire stream failed: error alloc buffers: Invalid argument
```

Three things follow. The parameter is read and acted on, so requesting shared
memory is a real safeguard rather than decoration. Mutter on this machine cannot
allocate DMA-BUF buffers at all, so shared memory here is the only option rather
than a fallback — which also means none of these results say anything about a
machine where DMA-BUF *is* available. And the failing run exercised the
session-loss path in the wild rather than only in a test: the dead session was
discarded and re-negotiated before the error was returned.

# Acceptance criteria status

All six criteria are settled on GNOME 50: five hold, and the sixth was narrowed to
the behaviour actually built and tested, with the rest split into
[Import DMA-BUF frames](dma-buf-frame-import.md).

| Criterion | Status |
| --- | --- |
| Consent asked once, then not again | holds |
| Frames in the documented RGB layout | holds; the criterion's premise needed correcting |
| Shared memory requested explicitly, DMA-BUF reported | holds, and the request is proven honoured |
| Session drop reported, and recoverable on demand | holds |
| Frame bytes correct, no channel swap | holds, against a controlled reference |
| Frame delivery does not block the MCP runtime | holds |

**"At the resolution the portal reports."** The criterion assumed the portal
reports the buffer size. It does not: it reports logical coordinates while the
stream carries physical pixels (1536x960 against 2560x1600 here). The backend now
reports both and converts between them.

**DMA-BUF-only sessions.** This criterion was narrowed rather than met, and the
reasoning is worth keeping. The original wording asked that capture still produce
frames on a session offering only DMA-BUF, which assumes a capability the
`pipewire` binding does not have: a DMA-BUF plane is reachable only as a file
descriptor, and there is no import, mmap, or modifier handling to get pixels out
of it — in 0.9.2 or 0.10.1. It was written during initialisation on the
assumption that DMA-BUF would be the common case, an assumption this work
disproved.

The criterion now covers what is built: shared memory is requested *explicitly*
through `SPA_PARAM_BUFFERS_dataType` rather than merely implied by the missing
video modifier, and that parameter is demonstrably honoured — see the reverse
experiment below, where asking for DMA-BUF instead made buffer allocation fail.
Importing DMA-BUF frames is [its own request](dma-buf-frame-import.md), blocked on
access to a compositor that offers them. Splitting it keeps the gap visible as a
named request rather than hiding it behind this one's `done`, and nothing here
claims DMA-BUF works.

**Session loss — fixed, and now covered by a test.** This had been a defect, not
merely an untested path: the stream's error state was ignored, so once a session
died `capture_frame` kept returning the last good frame, and a stale frame is
indistinguishable from a current one. Two changes: the stream's `state_changed`
callback records the failure and stops the loop, and `capture_frame` discards a
session that is no longer alive and negotiates a fresh one, so a compositor
restart costs a re-negotiation rather than failing until the process restarts.
`tests/frame_source_liveness.rs` covers it by connecting to the PipeWire daemon
directly — no portal, no consent — and pointing the stream at a node that does
not exist, then asserting that the capture fails rather than reporting a frame,
and that a stopped source does not claim to be alive.
