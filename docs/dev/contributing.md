---
type: Reference
title: Contributing
description: How a change moves from proposal to done in this project.
tags: [dev, conventions]
status: draft
---

# Contributing

## The workflow

A change request is a file in [backlog/](backlog/index.md), and it moves through
the states those documents define:

1. **Propose.** Write the change request with its problem, proposal, decisions,
   and acceptance criteria, in state `proposed`. Features, bug fixes,
   improvements, and refactors all start here.
2. **Plan.** Add the technical approach, the files to touch, and an ordered step
   checklist, and move it to `planned`.
3. **Implement.** Follow the plan, keep it updated as reality intrudes, run the
   build and tests, and move it to `done` when the acceptance criteria hold.
4. **Record.** Add a dated entry to the [changelog](changelog.md) describing
   what shipped.

A request that is no longer wanted moves to `rejected`; one that another request
replaces moves to `superseded`. Both are terminal and stay in the backlog as
history. The `kind`, `state`, and `priority` values are defined in the
[backlog index](backlog/index.md).

## Quality bar

- Every change request is grounded in a requirement in
  [../requirements.md](../requirements.md), or it names the new requirement it
  introduces.
- Acceptance criteria are testable and concrete: someone other than the author
  must be able to decide whether they hold.
- The build and the test suite pass before a request is marked `done`.
- A behaviour change updates the documents it affects. Structural changes to the
  documentation itself are recorded in the bundle's [log](../log.md), while
  shipped changes are recorded in the [changelog](changelog.md).
