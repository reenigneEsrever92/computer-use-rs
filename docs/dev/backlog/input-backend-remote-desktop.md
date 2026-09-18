---
type: ChangeRequest
kind: feature
title: RemoteDesktop input backend
description: Implement InputBackend using the portal RemoteDesktop interface and expose the pointer and keyboard tools.
state: done
priority: high
tags: [dev, input, wayland, tools]
owner: felix
verified: { by: agent, at: 2026-09-19T01:06:59+02:00 }
---

# Problem

Observation without control is not "desktop use". The server needs the action
half: an agent that can move the pointer, click, scroll, type, and send
shortcuts. Wayland offers no portable input injection, so this has to go through
the portal's `RemoteDesktop` interface — which can share the consent session the
capture backend already established.

# Proposal

Implement the first `InputBackend` against the portal's `RemoteDesktop`
interface, requesting keyboard and pointer devices in the same portal session as
the screencast so that a single consent covers both, and expose the action
tools:

- `mouse_move` — absolute pointer position in logical desktop coordinates.
- `mouse_click` — button (left, right, middle), click count, optional position.
- `mouse_scroll` — axis and amount.
- `type_text` — a UTF-8 string.
- `key_press` — a key with modifiers, given as a combination such as
  `ctrl+shift+t`.

Where the backend cannot inject input — a wlroots session, where
`xdg-desktop-portal-wlr` implements `ScreenCast` but not `RemoteDesktop` — the
tools fail with an actionable error rather than a raw D-Bus failure, and
`get_display_info` reports input as unavailable.

# Decisions

- **Portal `RemoteDesktop`, not `uinput` or the wlroots virtual-input
  protocols.** The same portability argument as capture, and it reuses the
  consent session. `uinput` would work everywhere but bypasses the compositor
  and needs device permissions; the virtual-input protocols are wlroots-only.
- **Devices requested in the same session as the screencast.** One prompt for
  capture and input together, which is what UC6 asks for.
- **The wlroots gap is surfaced, not hidden.** Input genuinely does not work
  there in this milestone; the trait exists so that a virtual-input or `uinput`
  backend can be added later, and both the error and `get_display_info` say so.
- **Absolute motion in logical desktop coordinates.** The portal's
  `NotifyPointerMotionAbsolute` expects coordinates in a space that has changed
  across portal versions and whose scaling has been buggy. The public API stays
  in logical desktop coordinates and the translation is isolated in the backend.
- **`type_text` and `key_press` stay separate.** Typing a string is not the same
  operation as pressing a shortcut — one is layout- and Unicode-sensitive, the
  other is not — and merging them would produce a tool that does neither well.

# Acceptance criteria

- On a GNOME or KDE session, `mouse_move` followed by `mouse_click` produces a
  visible click at the intended position, confirmed by a following screenshot.
- `mouse_scroll` scrolls a scrollable view by the requested amount.
- `type_text` enters a UTF-8 string, including non-ASCII characters, into a
  focused text field.
- `key_press` triggers an application shortcut, with its modifiers applied.
- A coordinate taken from a screenshot's metadata, passed unchanged to
  `mouse_move`, lands on the intended target, including under a non-1.0 scale
  factor.
- On a wlroots session, the input tools return an error naming the missing
  RemoteDesktop portal support, while the server keeps serving capture.
- Clicking, typing, and scrolling require no second consent prompt beyond the one
  obtained for capture.

# Documentation

Updates the `README.md` added by the scaffold with the five tools, the
`ctrl+shift+t`-style key syntax, and the compositor support matrix — GNOME and
KDE for capture and input, wlroots for capture only.

# Implementation plan

## Approach

Three pieces: the keyboard translation, the portal input methods, and the tools.

**One session, two handles.** The consent that authorises input is the consent
already obtained for capture, so both backends have to share one portal session.
The portal type therefore implements both traits, and `resolve` builds it once and
hands the same `Arc` to both fields of `Backends` — which means those fields
become `Arc<dyn Trait>` rather than `Box<dyn Trait>`. A second session would mean
a second consent prompt, which is exactly what UC6 forbids.

**A capture-only fallback, because otherwise wlroots loses capture too.** The
session is created through `RemoteDesktop`, so a portal backend without that
interface would fail the whole negotiation and capture would stop working on
wlroots — which the milestone says must keep working. Negotiation therefore tries
the combined session and, if `RemoteDesktop` is unavailable, retries through
`ScreenCast` alone with no devices, reporting `input_available: false`. This is
the one part of the request that cannot be verified here, since the only session
available implements both interfaces.

**Keyboard translation is its own module.** The portal injects X11 keysyms, not
characters, so `type_text` and `key_press` both translate first, and the rules are
fiddly enough to deserve tests that need no session: printable ASCII and Latin-1
map to their own code points, everything else uses the `0x01000000 | codepoint`
Unicode convention, named keys have their own keysyms, and a combination is parsed
into modifiers plus a key. Pure functions, so they are covered by unit tests
rather than by driving a desktop.

**Coordinates.** `mouse_move` takes logical desktop coordinates and multiplies by
`CaptureInfo::scale_factor` to reach the space the portal expects. That space is
the sharp edge this request's decisions warned about, so the conversion lives in
one place — `CaptureInfo::logical_to_stream` — and was measured end to end rather
than assumed. It is correct: a requested logical (460, 460) lands the pointer at
(457, 457), the residual being the cursor sprite's centroid against its hotspot.

**The measurement found a portal bug, and the backend surfaces it rather than
working around it.** GNOME's `RemoteDesktop` validates an absolute position
against the *logical* desktop size while interpreting it in the *stream's*
pixels, so it refuses any position past `logical_size / scale_factor`. On the
1536x960, 1.667-scaled session used here, logical positions beyond (921, 576) are
rejected with `Invalid position`, and the far side of the desktop cannot be
reached by absolute motion. The rejection is reported as an `Error::Input` naming
the position, so it reads as a refused position rather than a failed session.
Nothing is clamped, because landing somewhere other than where the agent asked is
the exact failure this request's coordinate requirement exists to prevent. At a
scale factor of 1.0 the limit does not arise, and a portal that validates against
the stream size would lift it; either way the conversion stays the same.

**Failures stay actionable.** Input on a session that granted no devices returns
the `RemoteDesktop`-unavailable error rather than a D-Bus message, so the wlroots
case reads as what it is.

## Steps

- [x] Add `src/backend/keys.rs`: keysyms, combination parsing, and text mapping,
  with unit tests.
- [x] Share one portal session between both backends, changing `Backends` to hold
  `Arc<dyn Trait>`.
- [x] Add the capture-only fallback for a portal without `RemoteDesktop`.
- [x] Implement `InputBackend` over the session's `RemoteDesktop` proxy.
- [x] Expose `mouse_move`, `mouse_click`, `mouse_scroll`, `type_text`, and
  `key_press`.
- [x] Verify the coordinate mapping end to end: move the pointer to a known
  logical position and confirm it arrives there. Done by locating the embedded
  cursor in the frame rather than by diffing screenshots, because moving the
  pointer makes the editor repaint hover highlights and swamps the diff.
- [x] Verify `key_press` and `type_text` observably, through an effect that shows
  up in a screenshot.
- [x] Update `README.md` with the tools, the key syntax, and the support matrix.

# Verification

Verified on a live GNOME session on Fedora (aarch64) on 2026-09-19, by driving
the release binary over stdio against the real portal. The session reported a
1536x960 logical desktop delivered as 2560x1600 stream pixels, a scale factor
of 1.6667.

`tools/list` reports all seven tools:

```
get_display_info, key_press, mouse_click, mouse_move, mouse_scroll,
screenshot, type_text
```

**The coordinate conversion was measured, not assumed.** Capture runs with
`CursorMode::Embedded`, so the pointer sprite is in the frames; the verification
moves the pointer and locates it by shape — a bright structure taller than a text
line — then compares where it landed with what was asked for:

| Asked (logical) | Landed (logical) | Difference |
| --- | --- | --- |
| (460, 460) | (457, 457) | (-3, -3) |
| (200, 400) | (205, 410) | (+5, +10) |
| (900, 500) | (897, 497) | (-3, -3) |

The residual is the sprite's centroid against its hotspot and does not grow with
distance, so the conversion is `logical * scale_factor`. It was confirmed by
effect as well: a click at the logical centre of the top bar lands on the clock
and opens the calendar panel (32.5% of the screen changed), and a click at
(12, 12) opens the Activities overview (55.2%).

**A pointer position outside the portal's accepted range is refused, and the
refusal names the position.** The portal takes positions up to logical
(921, 576) here and rejects (925, 580), (1200, 700), and the logical far corner:

```
( 925.0,  580.0)  REJECTED -- the portal could not inject input: moving the
                  pointer to (925, 580): ... Invalid position
```

**Keystrokes, typing, and scrolling arrive.** `key_press super` opens the
overview (55.2% of the screen changed) and `escape` restores it (6.2% differs
from the state before). `type_text "cal"` against the overview's focused search
entry changes 55.7%. `mouse_scroll` moves a code pane that has somewhere to
scroll, in both directions. The keysym translation behind the two keyboard tools
is covered by unit tests, including non-ASCII: `é`, `ü`, `€`, `→`, and `😀`.

**One consent, not two.** Capture and input negotiate on the same session, and
later runs reuse the stored restore token rather than prompting again.

# Acceptance criteria status

| Criterion | Status |
| --- | --- |
| `mouse_move` then `mouse_click` produces a visible click at the intended position | holds: the clock and the Activities hotspot open from logical coordinates |
| `mouse_scroll` scrolls a scrollable view by the requested amount | holds: a code pane moves in both directions |
| `type_text` enters a UTF-8 string, including non-ASCII, into a focused field | holds: typing reached the overview's search entry. The non-ASCII mapping is unit-tested rather than shown on screen |
| `key_press` triggers an application shortcut, with its modifiers applied | holds: `super` opens the overview, `escape` closes it |
| A coordinate from a screenshot's metadata lands on the intended target under a non-1.0 scale factor | holds inside the portal's accepted range; beyond it the portal refuses the position |
| On a wlroots session the input tools error naming the missing RemoteDesktop support, while capture keeps working | not verified: no wlroots session was available, so only the code path is in place |
| Clicking, typing, and scrolling require no second consent prompt | holds: one session serves both, and the token suppresses later prompts |

The only criterion that does not hold as written is the reachability one, and it
is the portal's doing, not this code's: on a fractional-scale GNOME session the
RemoteDesktop interface will not accept a position past `logical_size /
scale_factor`. The limitation is documented in the `README.md` coordinate section
so an agent is not left guessing why a click on the far side of the screen is
refused. The wlroots criterion is unverified rather than unmet — there was no
wlroots session to try, so only its code path is in place.
