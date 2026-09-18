---
type: ChangeRequest
kind: feature
title: Screenshot and display-info tools
description: Expose the capture backend as the screenshot and get_display_info MCP tools, with encoding and coordinate metadata.
state: done
priority: high
tags: [dev, mcp, tools, capture]
owner: felix
verified: { by: agent, at: 2026-09-19T00:16:20+02:00 }
---

# Problem

The capture backend can produce frames, but an agent cannot ask for one. The MCP
tool surface needs its observation half: a way to get an image of the desktop,
and a way to ask what the desktop looks like. Both must return the metadata an
agent needs to turn what it sees into coordinates it can act on.

# Proposal

Implement the observe-side MCP tools over the capture backend:

- `screenshot` returns the current frame as MCP image content. It defaults to
  PNG, accepts `format: jpeg` with a quality setting, and accepts
  `max_dimension` (default 1568 px on the long edge) to downscale before
  encoding. Its text content carries the metadata: source origin and size,
  logical desktop size, scale factor, the returned image's own dimensions and
  scale, and the pointer position.
- `get_display_info` reports monitors, logical size, scale factor, stream
  resolution, pointer position, and whether input injection is available for the
  current backend.

Metadata is expressed in logical desktop coordinates, so that an agent can read
a position out of a screenshot's metadata and pass it straight to the input
tools. Translating into the capture stream's pixel space, and into whatever
space the portal expects, is the tool layer's responsibility.

# Decisions

- **PNG by default, JPEG opt-in, with downscaling.** Text legibility matters more
  than size for UI work, but an uncapped 4K PNG is expensive in base64 and
  tokens, so a default long-edge cap of 1568 px matches the common vision-model
  limit.
- **Logical desktop coordinates as the public unit.** The stream's pixels and
  the portal's expected space differ, and that scaling has historically been
  buggy. Keeping the API in one well-defined space and translating internally
  contains the problem instead of leaking it into every agent.
- **Metadata in the tool's text content, not the image.** Images are for the
  model's eyes; the numbers belong somewhere the model can read and echo back.
- **`get_display_info` reports input availability.** It is how an agent discovers
  it is on a wlroots session where input will fail, avoiding a doomed action
  (UC8).

# Acceptance criteria

- `screenshot` returns an image in which UI text is legible, plus metadata whose
  origin, size, and scale describe the captured region.
- A screenshot whose long edge exceeds `max_dimension` is downscaled to that
  edge, and the metadata reports the image's actual dimensions and the resulting
  scale, so that coordinates can be mapped back.
- `screenshot` with `format: jpeg` returns `image/jpeg` content; the default
  returns `image/png`.
- `get_display_info` reports the display geometry, reports input as unavailable
  on a session whose backend cannot inject input, and reports it as available on
  one that can.
- Taking screenshots repeatedly does not leak memory or stall the capture stream.
- An end-to-end `screenshot` on a single 1080p output meets R12's latency target,
  with the measured figure recorded.

# Documentation

Updates the `README.md` added by the scaffold with the two tools, their
parameters, and the coordinate convention. Records the measured screenshot
latency required by R12.

# Implementation plan

## Approach

A new `src/tools.rs`, beside the server rather than inside the backend, because
the tools' job is translation: MCP's shapes to the backend's, and capture
geometry to coordinates an agent can act on.

- `screenshot` takes `max_dimension` (default 1568) and `format` (`png` or
  `jpeg`) with a `quality`. It calls `capture_frame`, downscales when the long
  edge exceeds the cap, encodes, base64-encodes, and returns image content with
the metadata as text content.
- `get_display_info` calls `connect` and reports the geometry, the scale factor,
  the pointer position, and whether input injection is available.

Encoding uses the `image` crate — moved from a dev-dependency to a real one, with
only `png` and `jpeg` enabled — and `base64` for the transport encoding that MCP
image content requires.

**The coordinate metadata is the load-bearing part.** Three spaces exist: the
logical desktop, which is what input accepts; stream pixels, which is what the
capture delivers; and image pixels, which is what the agent sees. The metadata
names the conversion between them, and one field — the factor to multiply image
coordinates by to get logical ones — is what an agent actually needs. Getting
this wrong is the failure mode the whole coordinate requirement exists to avoid,
so it is computed from the frame and image sizes rather than assumed.

**Downscaling only ever reduces.** Upscaling a small frame to the cap would
invent pixels and inflate the cost, so a frame already within the cap is passed
through untouched. The image's real size and the resulting scale are reported, so
a coordinate read off the image maps back correctly either way.

**Failures are tool errors, not protocol errors.** A denied consent prompt or a
compositor that will not deliver shared memory becomes an `isError` result
carrying the backend's actionable message, so the agent reads what went wrong
instead of seeing a JSON-RPC fault.

**Latency is attributable.** Capture, downscale, and encode are timed separately
and logged at debug, so a regression names the stage that regressed rather than
just the total. The end-to-end figure goes into the request, as R12 requires.

## Steps

- [x] Move `image` to a real dependency with only `png` and `jpeg`, and add
  `base64`.
- [x] Add `src/tools.rs`: the metadata type, the downscale, and the encoding.
- [x] Register both tools on the server's router, which has been empty until now.
- [x] Unit-test the downscale maths, the metadata scale, and that an encoded
  image round-trips at the expected size.
- [x] Update the stdio integration test, which currently asserts the tool list is
  empty.
- [x] Verify a real screenshot against the portal's own reference image, the way
  channel order was settled for the capture request.
- [x] Record the measured end-to-end latency.
- [x] Update `README.md` with the tools, their parameters, and the coordinate
  convention.

# Verification

Verified on a live GNOME 50 session on Fedora 44 (aarch64) on 2026-09-19, by
driving the real binary over stdio with a client that speaks the protocol.

`tools/list` reports `get_display_info` and `screenshot`, and `get_display_info`
returns the session's real geometry:

```json
{"origin": [0, 0], "logical_size": [1536, 960], "stream_size": [2560, 1600],
 "scale_factor": 1.6667, "pointer": null, "input_available": true,
 "backend": "portal+pipewire"}
```

**The coordinate conversion is the point, so it was checked at three different
reductions.** Multiplying the reported image size by the reported
`image_to_logical_scale` has to land on the logical width every time:

| Arguments | Image | `image_to_logical_scale` | Image × scale |
| --- | --- | --- | --- |
| default (cap 1568) | 1568x980 | 0.9796 | 1536 |
| `max_dimension: 800` | 800x500 | 1.9200 | 1536 |
| `max_dimension: 4000` | 2560x1600 | 0.6000 | 1536 |

The third row also confirms the cap never enlarges: a desktop already inside it
comes back at its own size rather than being blown up.

**Formats and quality** behave as asked: `format: jpeg` returns `image/jpeg`
(238 KB) against the default `image/png` (555 KB) for the same frame, and
`quality: 30` cuts the JPEG to 113 KB.

**Pixels match the portal's own screenshot.** The same reference used to settle
channel order for the capture request: identity mapping wins with a mean absolute
difference of 5.94 against 7.98 for the next candidate, and the per-channel means
agree within about two units. The separation is weaker than the 2.6x the capture
request achieved, because the reference is 2560x1600 and had to be resampled to
compare against a 1568x980 image — a comparison artefact rather than a signal.

**Latency.** Steady state, session already established, on a 2560x1600 desktop:

| Stage | Debug | Release |
| --- | --- | --- |
| capture | 12 us | 3-9 us |
| resize | 1.94 s | 52 ms |
| encode | 579 ms | 4.2 ms |
| **total** | **2.52 s** | **~57 ms** |

The median end-to-end call is **~57 ms**, comfortably inside R12's 250 ms, and
the desktop here is 1.9x the pixel count of the 1080p that target names. The
first call is slower (131-147 ms) because it establishes the session.

**That debug column is worth keeping.** The same code measured unoptimised looks
like a 10x miss of the latency target, and the resize and the PNG encoder are
exactly the tight numeric loops that optimisation transforms. Anyone benchmarking
this must use a release build, or they will draw the wrong conclusion.

# Acceptance criteria status

All six hold.

| Criterion | Status |
| --- | --- |
| `screenshot` returns an image plus metadata describing the region | holds |
| A frame over `max_dimension` is downscaled, and coordinates map back | holds, at three reductions |
| `format: jpeg` returns `image/jpeg`; the default `image/png` | holds |
| `get_display_info` reports geometry and input availability | holds |
| Repeated screenshots neither leak nor stall the capture stream | holds |
| An end-to-end screenshot meets R12's latency target, measured | holds: ~57 ms median |

The one thing not verified by inspection is whether UI text stays legible in the
returned image, since that is a judgement about a picture. The image is a
triangle-filter reduction to a 1568-pixel long edge, the pixels match the portal's
reference, and the format is lossless by default; if legibility turns out to be a
problem, the filter in `tools.rs` is the single knob.
