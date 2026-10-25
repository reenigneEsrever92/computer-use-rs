---
type: ChangeRequest
kind: bug
title: Recover input when the portal session closes
description: Input tools keep failing with a raw portal error after the shared portal session closes, where capture renegotiates and carries on.
state: proposed
priority: medium
tags: [dev, bug, input, portal, wayland]
owner: felix
---

# Problem

Capture and input share one portal session, and it can be closed underneath the
server: the compositor restarts, the captured source is removed, or the user ends
screen sharing. Capture already survives that. `capture_frame`
(`src/backend/portal.rs`) notices a frame source that is no longer alive,
discards the session and negotiates a fresh one, so a restart costs a
re-negotiation rather than failing until the process restarts.

Input has no such path. `pointer_move`, `pointer_button`, `pointer_scroll`, and
`key` call `ensure()`, send the event, and surface whatever the portal said.
`ensure()` opens a session only when the cached one is `None`, so once the
session is closed every input call keeps failing for the life of the process —
and what it reports is the portal's raw D-Bus string rather than a diagnosis.
R10 asks for a message that names what is missing rather than a raw D-Bus error,
and UC8 asks that the operator can tell what happened.

Seen on 2026-09-19 against a live GNOME session (mutter 50.4,
xdg-desktop-portal 1.22.1): `mouse_move` to an in-range logical position failed
with `the portal could not inject input: moving the pointer to (900, 500):
Portal request failed: org.freedesktop.zbus.Error: Invalid session`, three times
in a row, while a `screenshot` in the same process succeeded. Input worked again
on the call after that screenshot. The pattern is consistent with capture having
replaced the dead session — an inference, since the server's stderr was not
captured — but the input tools themselves never recover, which is the defect
either way. What closed the session was not determined: the portal and
gnome-shell had both been running since before the server started.

# Reproduction

Manually, on a live Wayland session:

1. Start the server and call any tool, so the portal session is established.
2. Close the session out from under it — end screen sharing from the desktop,
   or restart `xdg-desktop-portal`.
3. Call `mouse_move` with an in-range `coordinate`.

Observed: it fails with the portal's `Invalid session` error and keeps failing on
every later call. Expected: the next call negotiates a new session and succeeds.

Automated: recovery cannot be reproduced in the existing tests — `tests/stdio.rs`
spawns the server against a fake session environment and never reaches a portal,
and `PortalSession` talks to a live portal in one piece. The fix therefore
introduces a seam (a source of "is the current session still usable", in the
spirit of `FrameSource::is_alive`) and the test lands with it: a fake that
reports the session closed, a `pointer_move`, and an assertion that the backend
rebuilt the session and delivered the event to the new one. No automated test,
with manual verification only, was considered and rejected.

# Proposal

Give input the recovery capture has, driven by the session's own closed signal.
`ashpd`'s `Session::receive_closed()` yields the portal's `Closed` signal, so a
task per established session waits on it and, when it fires, clears the cached
`Established`. The next input call then finds none and negotiates a fresh
session, reusing the stored restore token, so recovery does not prompt again.

The call that was in flight when the session closed still returns an error; the
recovery is for later calls, not a transparent replay. That error should say
what happened — that the portal session was closed and that the next call will
establish a new one — instead of the raw D-Bus string, so UC8 holds for this
failure as it does for the others.

# Decisions

- **Recover on the portal's `Closed` signal, not by probing.** The signal says
  exactly when the session died, so nothing is polled and no call has to fail
  first; `ashpd` already exposes it as `Session::receive_closed()`. The
  alternative — mirroring capture by checking liveness after a failure — delays
  recovery by one failed call and misses a session that dies while idle.
- **No transparent replay of the failed event.** Recovery applies to the next
  call. Replaying a press that a hold sequence depends on could duplicate it, and
  the simplest rule — the call that met the dead session fails, the next one
  works — is the one that is easy to describe and to rely on.
- **The error names the cause.** A closed session is reported as such rather
  than as the portal's D-Bus message, which R10 and UC8 both ask for.
- **A seam, so the fix is testable.** The session lifecycle is factored so a fake
  can report a closed session without a portal. Without it the only exercise is a
  live session, which is not a regression test.

# Acceptance criteria

- After the portal session closes, the next input call establishes a new session
  and succeeds: a `mouse_move` to an in-range position followed by a `left_click`
  has its effect, confirmed by a following screenshot.
- The call that met the closed session reports that the session was closed, not
  the portal's raw D-Bus error.
- Recovery reuses the stored restore token, so it prompts for consent no more
  often than before.
- A capture-only session (wlroots) is unaffected: its input tools still report
  the missing `RemoteDesktop` interface.
- The unit test against the fake passes, and the build and the suite pass.

# Documentation

The README's Consent section already explains that a revoked token makes the
portal reject it and the server ask anew; recovering from an ordinary session
close is the same idea, so it deserves a sentence there. No other document is
affected.
