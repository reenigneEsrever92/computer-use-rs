---
type: ChangeRequest
kind: improvement
title: Document the pointer position limit and its real component
description: Correct where the scaled-desktop pointer limit comes from, record the reachable region, and note the workaround that is deliberately not taken.
state: proposed
priority: low
tags: [dev, improvement, docs, coordinates, wayland]
owner: felix
---

# Problem

The README's Coordinates section documents a real limitation — on a scaled
desktop, part of the screen is out of the portal's reach — but attributes it to
"GNOME's `RemoteDesktop` portal". The check is in neither the GNOME portal
backend nor Mutter. It is `check_position()` in the xdg-desktop-portal
*frontend* (`src/remote-desktop.c`), which rejects any absolute position outside
the size the backend reported for the stream:

```c
screen_cast_stream_get_size (stream, &width, &height);
if (x >= 0.0 && x < width && y >= 0.0 && y < height)
  return TRUE;
return FALSE;   /* -> "Invalid position" */
```

On GNOME the backend reports the monitor's *logical* size — 1536x960 — while the
frames, and the coordinates Mutter consumes, are in video pixels, 2560x1600. The
two disagree under fractional scaling, so every position past the logical size is
refused before Mutter sees it, and absolute motion reaches only the top-left
`logical_size / scale_factor`.

The attribution matters. The frontend check is shared by every backend, so this
is not GNOME-specific: whether it bites depends on the size a backend reports for
a scaled stream. A reader who takes the README at face value looks for the fix in
the wrong component, and is not told that a scale factor of 1.0 is unaffected.

Re-verified on 2026-09-19 (mutter 50.4, xdg-desktop-portal-gnome 50.0,
xdg-desktop-portal 1.22.1) on a 1536x960 logical, 2560x1600 stream session with a
scale factor of 1.6667. The server sends `logical * scale`, and the boundary
falls exactly at the logical size:

| Logical asked | Stream sent | Result |
| --- | --- | --- |
| (900, 500) | (1500.0, 833.3) | accepted |
| (921, 575) | (1535.0, 958.3) | accepted |
| (921.6, 575) | (1536.0, 958.3) | rejected — `Invalid position` |
| (922, 576) | (1536.7, 960.0) | rejected |
| (1535, 959) | (2558.3, 1598.3) | rejected |

Clicking logical (768, 15) opens the clock at the top bar's logical centre, and
the server sent 1280 for it, so the number is read as a video pixel: the
`logical * scale` conversion is right and the frontend's cap is the defect.

This is the known gap of R6 — normalizing coordinates across the logical
desktop, the stream's pixels, and the scale factor — and of UC9. The README is
where that gap is documented, so this request is about making the documentation
accurate: naming the component a future fix would have to involve, and the region
the limit leaves reachable.

# Proposal

Correct the section so it names the component that does the checking, states the
mismatch that causes it (a reported logical size against a scaled stream), gives
the reachable region as `logical_size / scale_factor`, and records that a scale
factor of 1.0 is unaffected.

Record in the same place the workaround that is available and deliberately not
taken: only the absolute call is checked, so a relative `NotifyPointerMotion`
could reach the far side. The shape it should take, if it is ever wanted, is an
absolute move to a reachable anchor inside the accepted band followed by a
relative move for the remainder — reliable only because the anchor pins the
starting point, since the portal never reports where the pointer actually is. It
stays out of scope here.

# Decisions

- **Documentation only.** The behaviour is right as it stands: the server reports
  the refused position rather than moving somewhere the agent did not ask for,
  and the portal's check, not this code, is the cause. So this is an improvement
  to the documentation, not a code change.
- **Name the frontend, not GNOME.** The check is `check_position()` in
  xdg-desktop-portal's frontend and runs for every backend; the GNOME-specific
  part is that Mutter reports a logical size for a scaled stream.
- **Record the relative-motion fallback as an option, not a plan.** It is written
  down so a later request can pick it up, and explicitly not implemented, because
  it depends on a pointer position the portal does not report.
- **Update the deferred list too**, so the option is discoverable from
  `docs/requirements.md` as well as the README.

# Acceptance criteria

- The README's Coordinates section attributes the check to xdg-desktop-portal's
  frontend `check_position()`, and no longer implies it belongs to GNOME's portal
  backend.
- It states the cause as a reported logical size against a scaled stream's video
  size, gives the reachable region as `logical_size / scale_factor`, and says a
  scale factor of 1.0 is unaffected.
- `docs/requirements.md`'s Deferred list records the relative-motion fallback as
  a considered but unadopted option.
- No code changes; the build and the suite are unaffected.

# Documentation

- `README.md` — the Coordinates section's "Known limitation" paragraph.
- `docs/requirements.md` — the Deferred list.
