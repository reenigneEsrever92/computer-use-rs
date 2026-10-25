# Backlog

Change requests live here, one file per request. Each states the problem, the
proposal, the decisions behind it, and its acceptance criteria; an implementation
plan is added before work starts.

## The first milestone

These five requests deliver the first milestone. The order is the intended
sequence of work; it is not the same as priority, because later work depends on
earlier work — for example, the preview window is medium priority but is
sequenced early, since it only needs the capture backend and makes the rest of
the milestone observable.

| Order | Change request | Kind | Priority | State |
| --- | --- | --- | --- | --- |
| 1 | [Scaffold the desktop-use server](scaffold-desktop-use.md) | feature | high | done |
| 2 | [Portal and PipeWire capture backend](portal-pipewire-capture-backend.md) | feature | high | done |
| 3 | [Preview window](preview-window.md) | feature | medium | done |
| 4 | [Screenshot and display-info tools](screenshot-display-info-tools.md) | feature | high | done |
| 5 | [RemoteDesktop input backend](input-backend-remote-desktop.md) | feature | high | done |

Milestone scope, and what is deliberately deferred, are defined in
[requirements.md](../../requirements.md).

## Follow-ups

Work that was split out of a milestone request, or that the milestone deliberately
did not cover, and that is not needed to demonstrate the project's value:

| Change request | Kind | Priority | State |
| --- | --- | --- | --- |
| [Expand the tool surface](expand-tool-surface.md) | feature | high | in-progress |
| [Import DMA-BUF frames](dma-buf-frame-import.md) | feature | medium | proposed |
| [Recover input when the portal session closes](recover-closed-portal-session.md) | bug | medium | proposed |
| [Document the pointer position limit](document-pointer-position-limit.md) | improvement | low | proposed |
| [Rename the project to computer-use](rename-project-to-computer-use.md) | refactor | medium | done |

[Expand the tool surface](expand-tool-surface.md) is the follow-up the first
milestone left: its action tools pressed and released within a single call, so the
desktop could be clicked but not dragged, selected, or held. It is implemented and
its tests pass, and is `in-progress` only until the behaviour is confirmed on a
live session.

[Import DMA-BUF frames](dma-buf-frame-import.md) was split out of the capture
backend rather than left to block it: the criterion it came from asked for frames
from a compositor offering only DMA-BUF, which no available producer can exercise.
It stays `proposed` until there is a session to develop and test against.

## `kind`

- `feature` — new behaviour.
- `bug` — existing behaviour that is wrong.
- `improvement` — existing behaviour made better.
- `refactor` — the same behaviour, restructured.

## `state`

- `proposed` — written and awaiting a plan.
- `planned` — has an implementation plan and is ready to start.
- `in-progress` — being worked on.
- `done` — acceptance criteria hold and the changelog records it.
- `rejected` — decided against; kept as history.
- `superseded` — replaced by another request.

The workflow that moves a request between these states is described in
[../contributing.md](../contributing.md).
