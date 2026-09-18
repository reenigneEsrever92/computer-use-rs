---
type: ChangeRequest
kind: feature
title: Import DMA-BUF frames
description: Read frames from a compositor that offers only DMA-BUF buffers, which the shared-memory path cannot.
state: proposed
priority: medium
tags: [dev, capture, wayland, pipewire]
owner: felix
---

# Problem

Capture reads pixels out of PipeWire buffers that live in shared memory. A
compositor that offers **only DMA-BUF** buffers cannot be served at all.

The reason is not a missing branch but a missing capability in the binding we use.
For a DMA-BUF buffer, `pipewire` 0.9.2 hands over a valid file descriptor and the
plane geometry, and `Data::data()` returns `None`: the pixels are reachable only
through that descriptor. The crate provides no mmap, no DMA-BUF import, no plane
or modifier handling, and that is equally true of 0.10.1. Importing is code this
project would have to write, not configuration it can switch on.

Today capture detects the situation and stops with an error naming the limitation.
That is honest, but it means capture does not work on such a session, so the claim
that the capture backend is compositor-agnostic holds only for compositors that
offer shared memory.

**This request is blocked on the environment rather than on design.** No producer
available on the development machine offers DMA-BUF: GNOME/Mutter negotiates
shared memory, and fails outright when asked for DMA-BUF instead
(`error alloc buffers: Invalid argument`). A KDE (KWin) or wlroots session is the
likely trigger, and until one is available there is nothing to develop against and
nothing to test with.

# Proposal

Import DMA-BUF frames and convert them to packed RGB, so that a session offering
only DMA-BUF behaves the same as one offering shared memory.

Two candidate approaches, in the order they are worth trying:

- **Map the plane directly where the producer allows it.** A DMA-BUF plane can
  carry `SPA_DATA_FLAG_MAPPABLE`, in which case the buffer may already be readable
  through `MAP_BUFFERS`. `libspa` 0.9.2 exposes no constant for that flag in
  `DataFlags`, though the raw flag exists, so this would be tested through
  `Data::as_raw()`. If it works it is far cheaper than the alternative.
- **Import through EGL or GBM and read back.** Bring the descriptor into a GPU
  context, copy the pixels back to system memory, and hand them to the existing
  conversion in `backend/convert.rs` unchanged.

The compositor should continue to be asked for shared memory first: the explicit
`SPA_PARAM_BUFFERS_dataType` request already does this, and that request is
verified to be honoured. DMA-BUF import is a fallback for producers that refuse,
not a preference, because this project encodes on the CPU and has no use for a
zero-copy GPU path.

Whatever the path, the frame source should report which memory type it used, so
the existing diagnostic stays truthful and a future report of "it works" can say
on what.

# Decisions

- **Split out of the capture change request rather than expanding it.** That
  request's own risk note said DMA-BUF work "becomes its own change request rather
  than silently expanding the milestone", and this is that request. The split was
  not triggered by failure: GNOME negotiated shared memory, so the milestone's
  target platform never needed it.
- **The gap stays visible.** Removing the criterion from the capture request would
  have hidden a real portability gap behind a "done"; this request exists so that
  someone looking for DMA-BUF support finds a request naming it.
- **A CPU readback, not a GPU pipeline.** The frames are destined for PNG/JPEG
  encoding on the CPU, so a zero-copy GPU path would be optimising a step that does
  not exist. The importer's job is to get pixels where the converter can see them.
- **Shared memory is still requested first.** The explicit request is cheaper than
  an import and is already proven to be honoured, so it stays the primary path.
- **Not planned until a producer exists.** There is no value in a plan that cannot
  be executed or verified, so this request stays `proposed` until a session that
  offers only DMA-BUF is available.

# Acceptance criteria

- On a session whose compositor offers only DMA-BUF, capture produces correct RGB
  frames, checked against the portal's own screenshot as an independent reference
  — the method used to settle channel order for the shared-memory path.
- The frames match the shared-memory path's output for the same content, so the
  two paths are interchangeable rather than merely both non-empty.
- The frame source reports which memory type it used.
- Where shared memory is available, behaviour is unchanged.
- A DMA-BUF session whose buffers still cannot be read fails with an actionable
  error rather than a hang or a blank frame.

Note that none of these can be verified until a qualifying session exists; the
request is blocked on availability, not on design, and should not be marked
`done` on the strength of a compile alone.

# Documentation

Updates `README.md`, which describes the buffer-type diagnostic and would need to
stop presenting DMA-BUF as unsupported. If the fallback introduces a build or
runtime dependency (EGL, GBM), the prerequisites section changes too. Updates
[requirements.md](../../requirements.md) to move DMA-BUF import out of its
deferred list.
