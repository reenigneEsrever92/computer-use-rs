---
type: ChangeRequest
kind: feature
title: Expand the tool surface with held, modified, and timed interaction
description: Align the MCP tools with the computer-use vocabulary and add held buttons and keys, modifier-bearing clicks, drag, zoom, and wait.
state: in-progress
priority: high
tags: [dev, mcp, tools, input]
owner: felix
---

# Problem

Every action tool is a whole action. `mouse_click` presses and releases a button
inside one call, and `key_press` does the same for a key, so there is no way to
hold either across calls. That rules out drag and drop, text selection, window
resizing, and drawing — and, because `mouse_click` carries no modifier, it also
rules out the shift-click and ctrl-click that a range or multi-select needs. The
click `count` reaches double and triple clicks and nothing else.

Observation is thin in the same way. `screenshot` downscales the whole desktop
to the cap, so exactly the small text an agent needs — file names, tab titles,
status-bar text, button labels — is the text that goes illegible, with no way to
ask for one region at full resolution instead.

Waiting is absent, so an agent that must let an application settle has no choice
but to sleep blindly, which is the largest single source of races in a
tool-calling loop.

The surface also diverges from the vocabulary agents are already trained on.
Anthropic's computer use toolset (`computer_toolset_20260801`) names its members
`left_click`, `left_click_drag`, `left_mouse_down`, `left_mouse_up`, `scroll`,
`key`, `type`, `hold_key`, `wait`, `zoom`, and `cursor_position`. This server
uses near-synonyms for several of them and lacks the rest, which costs click
accuracy in exchange for nothing.

# Proposal

Adopt the computer use member names and input shapes, and fill the gaps they
name. Coordinates remain logical desktop coordinates throughout — the one
deliberate divergence, explained under Decisions.

**Observe**

- `screenshot` — unchanged: the full desktop, with `max_dimension`, `format`,
  and `quality`.
- `zoom` — `region: [x0, y0, x1, y1]` in logical coordinates. The region is
  captured at full resolution and scaled to fit the cap, and the same metadata
  shape as `screenshot` is returned, so a position read off the zoomed image
  maps back to the logical desktop. This is the legibility tool: the cap is spent
  on a small area instead of the whole screen.
- `cursor_position` — the pointer's logical position, as text.
- `get_display_info` — unchanged. It has no computer use equivalent and carries
  the geometry and the `input_available` flag that UC8 relies on.

**Pointer**

- `mouse_move` — `coordinate: [x, y]`.
- `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click` —
  optional `coordinate`, optional `modifiers`. Double and triple are the left
  button, clicked twice and three times.
- `left_click_drag` — `start_coordinate`, `coordinate`, optional `modifiers`.
- `left_mouse_down`, `left_mouse_up` — the button held across calls, for the
  drags `left_click_drag` cannot express.
- `scroll` — `scroll_direction` (`up`, `down`, `left`, `right`),
  `scroll_amount` in wheel steps, optional `coordinate`, optional `modifiers`.

`mouse_click` and `mouse_scroll` are removed: their behaviour is now the click
tools and `scroll`.

**Keyboard**

- `type` — `text`, replacing `type_text`.
- `key` — `text` (a key or a `+`-joined combination such as `ctrl+s`), optional
  `repeat` (1–100), replacing `key_press`.
- `hold_key` — `text`, `duration` in seconds: press, wait, release, atomically.

**Timing**

- `wait` — `duration` in seconds, up to 300.
- `wait_for_change` — `timeout` in seconds, optional `min_change`: returns as
  soon as the screen has changed, or reports that it had not by the timeout.

`modifiers` accepts what `keys.rs` already parses — `shift`, `ctrl`, `alt`,
`super`, or a `+`-joined combination — and holds them only for the duration of
that action, the way computer use does.

The backend's methods are action-shaped, so they cannot express held state. The
`InputBackend` trait therefore gains press and release primitives for pointer
buttons and keysyms. Everything composed — click counts, double and triple
clicks, drag interpolation, modifier-holding, `hold_key` timing, and the
text/combination translation `keys.rs` already provides — is layered above them,
and existing behaviour such as the capture-only fallback is untouched.

# Decisions

- **One request, not two.** The input and observation changes share one
  justification (the surface is action-shaped where it needs to be stateful or
  scoped) and one test approach (drive a real session and confirm by
  screenshot), so splitting them would duplicate both.

- **The computer use vocabulary, names and input shapes.** Tool names and
  argument shapes follow `computer_toolset_20260801`: `left_click`,
  `left_click_drag`, `left_mouse_down`/`left_mouse_up`, `scroll`, `key`, `type`,
  `hold_key`, `wait`, `zoom`, `cursor_position`. A model that has seen that
  surface transfers to this one; near-synonyms cost accuracy for no benefit. The
  consequence is that `mouse_click`, `mouse_scroll`, `type_text`, and `key_press`
  are renamed or split, and `mouse_click`'s `count` gives way to the named
  `double_click` and `triple_click`. The project is at 0.1.0 with no released
  users, so the break costs nothing and this is the moment to take it.

- **Coordinates stay logical desktop coordinates, not screenshot pixels.** This
  is the one deliberate divergence. Computer use expresses every coordinate in
  the pixel space of the screenshots it returns. Here, logical desktop
  coordinates are what the input backend takes, what `get_display_info` reports,
  and what the screenshot metadata already maps onto through
  `image_to_logical_scale`. Tying the input tools to screenshot pixels would make
  every click depend on the scale of the last screenshot, reintroducing exactly
  the coordinate ambiguity the first milestone set out to eliminate. The README
  already documents the convention, and it stays.

- **Modifiers are a parameter on the action, not a held-key tool.** Shift-click,
  ctrl-click, and shift-drag are expressed with `modifiers` on the click, drag,
  and scroll tools, which is how computer use does it, rather than by exposing a
  general keyboard down/up pair. It is one call instead of three, and it cannot
  leave a modifier stuck down if the caller stops midway. The name parser in
  `keys.rs` already resolves these, so nothing new is needed to read them.

- **No `key_down`/`key_up`; `hold_key` covers timed holds.** Computer use
  deliberately omits raw keyboard down/up, and the cases that would need them
  split in two: a modifier around an action is the `modifiers` parameter, and a
  key held for a while — the one that matters, because it drives auto-repeat — is
  `hold_key`, which is atomic and cannot outlive its call. A held-state API that
  can dangle is not worth adding for a case these two already cover.

- **`left_mouse_down` and `left_mouse_up` are left-button, following computer
  use.** It is the button the press-and-hold case is about, and matching the
  vocabulary keeps the names predictable. Right and middle holds are the obvious
  extension if a use case turns up; the backend already takes any button.

- **`zoom` is its own tool, not a `region` argument on `screenshot`.** The two
  are different operations, not two spellings of one: `screenshot` reduces the
  whole desktop to the cap, while `zoom` crops a region and *then* fits it to the
  cap, which is what makes text legible. A distinct name is also what prompts an
  agent to reach for it when a full screenshot is illegible, which is the failure
  it exists to fix.

- **The modifier field is `modifiers`, not computer use's `text`.** Computer use
  overloads `text` to mean the modifier list on a click and the string on `type`.
  Here `text` already means the typed string for `type`, `key`, and `hold_key`,
  and reusing it for a list of modifiers would invite exactly the confusion the
  overload creates. The tool names still follow computer use; this one argument
  name does not.

- **`wait` and `wait_for_change` are both included.** `wait` is the computer use
  member and covers a fixed delay, and for the cases where nothing observable
  changes it is the only option. `wait_for_change` is strictly better whenever
  the trigger is observable — it removes the blind sleep that causes races —
  and it is what the deferred "wait tool" in
  [requirements.md](../../requirements.md) was really asking for. Including both
  promotes that deferred item rather than leaving it.

- **`wait_for_change` compares a threshold, not exact equality.** An embedded
  pointer and a blinking text caret both change pixels without changing the
  screen, so a change is defined as more than a small fraction of pixels
  differing, with the fraction configurable. Exact-equality detection would fire
  on a caret blink and be useless.

- **`get_display_info` stays.** It has no computer use counterpart, it carries
  information no other tool reports — the captured region's origin and logical
  size, the stream size, the scale factor, and whether input is available here —
  and UC8 depends on that last field to tell an agent it is on a capture-only
  session before it attempts a doomed action.

- **The backend gains press and release, and composition moves above the trait.**
  The trait currently exposes whole actions — a click with its count, a full
  keystroke — which is precisely the shape that makes held state impossible.
  Splitting buttons and keysyms into press and release is the smallest change
  that unlocks drags, holds, and modifier-bearing actions, and it keeps
  composition (counts, interpolation, timing) in one place rather than threading
  held state through the trait.

- **Out of scope, and why.** Clipboard read/write needs an access path the
  portal does not provide and so breaks the thin-wrapper shape. Launching
  applications and managing windows are outside driving the desktop that is
  already there, and computer use itself pairs with separate bash and editor
  tools rather than folding them in. Relative pointer motion is not in the
  computer use vocabulary and is covered by absolute motion plus
  `cursor_position`. Each is a candidate for its own request.

# Acceptance criteria

- `tools/list` reports the new surface — `screenshot`, `zoom`, `get_display_info`,
  `cursor_position`, `mouse_move`, `left_click`, `right_click`, `middle_click`,
  `double_click`, `triple_click`, `left_click_drag`, `left_mouse_down`,
  `left_mouse_up`, `scroll`, `type`, `key`, `hold_key`, `wait`,
  `wait_for_change` — and no longer reports `mouse_click`, `mouse_scroll`,
  `type_text`, or `key_press`.
- `left_click`, `right_click`, and `middle_click` deliver the named button at the
  named position, confirmed by a following screenshot.
- `double_click` and `triple_click` produce two and three clicks, confirmed by an
  effect that only they cause.
- `left_click_drag` performs a drag from `start_coordinate` to `coordinate`: a
  drag across text selects it, confirmed by a following screenshot.
- `left_mouse_down`, then `mouse_move`, then `left_mouse_up` performs a drag
  spanning several calls, for a case `left_click_drag` cannot express.
- A click with `modifiers: shift` extends a selection, and one with
  `modifiers: ctrl` toggles a single item in a multi-select, each confirmed by a
  following screenshot.
- `scroll` moves the view the way each `scroll_direction` names, in the amount
  asked for, and accepts a `coordinate` to scroll a particular element.
- `key` with `repeat: 4` presses four times, observed as four steps of the
  effect the key has.
- `hold_key` holds long enough to drive auto-repeat — a held arrow key scrolls a
  list continuously — and releases without leaving the key down.
- `type` enters a UTF-8 string including non-ASCII characters.
- `zoom` returns a region in which text illegible in a full screenshot is
  legible, and its metadata maps a position read off the zoomed image back to the
  logical coordinate that lands where the agent intends.
- `cursor_position` reports the last position this server commanded, and says
  the position is unknown before anything has moved the pointer. The portal
  reports no pointer position at all, so a last-commanded value is the most that
  can be offered, and it is reported as such rather than as a measurement.
- `wait` delays for the requested duration.
- `wait_for_change` returns promptly once the screen changes — for example after
  a click that triggers a delayed redraw — and reports that it did not change
  within the timeout when nothing changes; a blinking caret does not count as a
  change.
- The coordinate convention is unchanged: a position taken from `screenshot`
  metadata and passed to `left_click` lands on the intended target, including
  under a non-1.0 scale factor and within the portal's accepted range.
- None of the new tools triggers a second consent prompt.

# Documentation

Updates `README.md`: the tool table becomes the new surface, and the sections on
key combinations, coordinates, and the modifier convention are rewritten around
`key`, `type`, `hold_key`, and `modifiers`. The coordinate section keeps its
convention but gains a note that the names follow computer use while coordinates
remain logical desktop coordinates rather than screenshot pixels, so a reader
who knows that surface is not misled. The `zoom`, drag, hold, and wait tools are
documented with what each is for, not only its parameters.

Updates [requirements.md](../../requirements.md): R3's list of exposed tools and
R4's description of injected input both change — the latter gains modifiers,
drags, and holds — and UC2, UC4, and UC5, which name the old tools, are reworded.
The deferred `wait` tool is removed from [Deferred](../../requirements.md), since
this request delivers it.

Updates `docs/overview.md`, whose one-sentence list of actions gains dragging and
holding.

Adds this request to the backward index in
[docs/dev/backlog/index.md](index.md).

# Implementation plan

## Approach

The existing action-shaped backend methods — a whole click with its count, a whole
keystroke — are what make held state impossible, so the work starts there and
moves outward. Five files change and no new module is needed.

**`src/backend/keys.rs`** gains `parse_modifiers`, which turns `ctrl+shift` into
the keysyms the `modifiers` argument needs. It reuses the private
`modifier_keysym` the combination parser already has, so the two agree on what a
modifier is.

**`src/backend/input.rs`** shrinks the trait to the events the portal actually
takes — `pointer_move`, `pointer_button(button, state)`, `pointer_scroll`,
`key(keysym, state)` — and grows free functions that compose them: `click`,
`drag`, `scroll`, `type_text`, `press_combination`, and `hold_keys`. They take
`&dyn InputBackend`, so they are testable against a mock backend and hold the
modifier press/release-around-an-action logic in one place, including releasing
on failure. Composition moves here from the portal impl, which becomes a thin
wire layer.

**`src/backend/portal.rs`** implements the primitives. It also records the
pointer position an absolute move asked for on the session's `CaptureInfo`, so
the pointer fields stop being permanently `None`. This is the honest limit: the
portal's RemoteDesktop interface does not report where the pointer *is*, only
where it was told to go, so `cursor_position` reports the last commanded position
and says so when there is none.

**`src/tools.rs`** replaces the old argument types with the new ones, and adds
three pure helpers: `crop` for `zoom`, `changed_fraction` for `wait_for_change`,
and a region-aware `ScreenshotMetadata` constructor so a zoomed image's metadata
maps back to the logical desktop. Duration parsing (seconds, capped at 300) lives
here too, beside the defaults it clamps to.

**`src/mcp.rs`** registers the nineteen tools. Most are a few lines over the
composition helpers; the exceptions are `zoom`, which crops before it fits, and
`wait_for_change`, which polls `capture_frame` until `changed_fraction` crosses
`min_change`. The `type` member is a Rust method named `type_` carrying
`#[tool(name = "type")]`, which rmcp 1.8.0 supports.

**Tests.** `crop`, `changed_fraction`, the region metadata, and duration parsing
are unit-tested in `tools.rs`; the composition helpers are tested against a mock
`InputBackend` in `input.rs`; `tests/stdio.rs` asserts the listed surface. The
live-session criteria cannot be run without a session, so they are recorded as
the remaining verification, the way the earlier requests recorded theirs.

## Steps

- [x] Add `parse_modifiers` to `src/backend/keys.rs`, with tests for a valid list,
an unknown name, and the empty string.
- [x] Reduce `InputBackend` to press/release primitives and add the composition
  helpers in `src/backend/input.rs`, with a mock-backend test.
- [x] Update `src/backend/portal.rs` to the primitives and record the last
  commanded pointer position on `CaptureInfo`.
- [x] Replace the argument types and add `crop`, `changed_fraction`, the region
  metadata constructor, and duration parsing to `src/tools.rs`, with tests.
- [x] Register the nineteen tools in `src/mcp.rs`.
- [x] Update `tests/stdio.rs` to assert the new surface and the absence of the
  old tool names.
- [x] Rewrite the tool, key-combination, and coordinate sections of `README.md`
  for the new surface, and the new `modifiers` convention.
- [x] Update `docs/requirements.md`: R3's tool list, R4's description of injected
  input, the UC2/UC4/UC5 wording that names the old tools, and the deferred `wait`
  entry.
- [x] Update the action list in `docs/overview.md`.
- [x] Run `cargo build`, `cargo clippy --all-targets`, and `cargo test`.

# Verification

**Automated, and green.** `cargo build` and `cargo clippy --all-targets` are clean,
and `cargo test` passes 78 unit tests and 8 integration tests. The new coverage is
`crop`, `changed_fraction`, the region metadata, duration parsing, and the scroll
direction's sign in `tools.rs`; modifier parsing in `keys.rs`; and the composition
helpers — click counts, the modifier hold-and-release including the failure path,
drag interpolation, scrolling, typing, repeat, and hold — against a mock backend
in `backend/input.rs`. `tests/stdio.rs` asserts the nineteen listed tools and the
absence of the four replaced names.

**Not verified: everything that needs a live desktop.** The behavioural criteria
below require a GNOME or KDE session, driving the real binary as the earlier
requests were verified — a client over stdio against the real portal. They were
deliberately not run during implementation: every one of them moves the pointer,
clicks, drags, types, or holds a key, which is to say they take over whatever
desktop they are run on. That is not something to do to the machine the work is
being developed on without being asked first. The primitives they build on —
absolute pointer motion, button press and release, keysym press and release — were
verified live when the RemoteDesktop backend landed, so the untested part is the
composition and the portal plumbing around it, not the wire calls themselves.

# Acceptance criteria status

| Criterion | Status |
| --- | --- |
| `tools/list` reports the new surface and no longer the old names | holds: asserted by `tests/stdio.rs` |
| `left_click`, `right_click`, `middle_click` deliver the named button | not verified: needs a live session |
| `double_click` and `triple_click` produce two and three clicks | not verified: needs a live session |
| `left_click_drag` selects a range | not verified: needs a live session |
| `left_mouse_down`/`mouse_move`/`left_mouse_up` drags across calls | not verified: needs a live session |
| `modifiers` extends a selection and toggles a multi-select | not verified: needs a live session |
| `scroll` moves the view each way and accepts a coordinate | not verified: needs a live session |
| `key` with `repeat: 4` presses four times | not verified: needs a live session |
| `hold_key` drives auto-repeat and releases | not verified: needs a live session |
| `type` enters UTF-8 text, non-ASCII included | not verified: needs a live session. The keysym mapping is unit-tested; the live check is the one the RemoteDesktop request also ran |
| `zoom` makes illegible text legible and maps back | partially: crop and region metadata are unit-tested; legibility needs a live session |
| `cursor_position` reports the last commanded position | not verified: needs a live session |
| `wait` delays for the duration | not verified: needs a live session, though it is a sleep |
| `wait_for_change` returns on change and reports a timeout | partially: `changed_fraction` and the caret tolerance are unit-tested; the live timing needs a session |
| Coordinates are unchanged and still land under a non-1.0 scale factor | not verified: needs a live session |
| No new tool triggers a second consent prompt | not verified: needs a live session |

This request is `in-progress` rather than `done` for that reason alone. Closing it
needs one pass on a live GNOME or KDE session, confirming the criteria above and
recording the results here the way the earlier requests did.
