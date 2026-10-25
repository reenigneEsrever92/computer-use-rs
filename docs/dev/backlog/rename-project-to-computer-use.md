---
type: ChangeRequest
kind: refactor
title: Rename the project to computer-use
description: Rename the crate, binary, MCP server name, log filter, and state directory from desktop-use to computer-use.
state: done
priority: medium
tags: [dev, naming]
owner: felix
verified: { by: agent, at: 2026-09-19T14:13:46+02:00 }
---

# Problem

The project is called `desktop-use` everywhere it names itself: the crate and the
binary, the name the MCP server announces, the environment variable that sets the
log filter, and the directory the portal consent token is kept in. Its tool
surface, meanwhile, now follows Anthropic's *computer use* toolset — `screenshot`,
`left_click`, `left_click_drag`, `scroll`, `key`, `type`, `hold_key`, `zoom`, and
the rest — so the tools say one thing and the program says another.

Two names for one project cost every reader a translation, and the cost grows with
every document that mentions it. It is at its cheapest to fix now, before any
release.

# Proposal

Rename the project to `computer-use` wherever the name is live:

- the package and the binary become `computer-use`, and the library target becomes
  `computer_use`, so every `use desktop_use::…` becomes `use computer_use::…`;
- the MCP server announces `computer-use`, which is what `env!("CARGO_PKG_NAME")`
  already reports once the package is renamed;
- the log filter becomes `COMPUTER_USE_LOG`;
- the consent token moves to `$XDG_STATE_HOME/computer-use/restore-token`;
- the preview window's title, the capture thread's name, the texture name, the
  `--help` name, and the messages in `error.rs` follow.

The dated records keep the name they were written under: the change requests
already in the backlog, the changelog entries already written, and the bundle log.
They are history, and rewriting the captured program output inside them would be
inventing evidence rather than recording it.

# Decisions

- **Rename the product, not the history.** Live artifacts — `src/`, `tests/`,
  `examples/`, `Cargo.toml`, `README.md`, and the current concept documents — carry
  the new name. The backlog requests already written, their slugs, the changelog
  entries, and `docs/log.md` keep the old one, because they are dated statements of
  what shipped when. The scaffold request's summary even embeds captured log lines
  reading `desktop_use::preview`, and rewriting those would misrepresent output that
  really was produced.

- **No compatibility aliases.** The project is at 0.1.0 with no released users, so
  the old environment variable is not kept as a fallback and the old state
  directory is not read. A rename that leaves both names working is two names to
  maintain, which is the problem this request exists to remove.

- **Moving the consent token re-prompts once.** The token lives under the state
  directory, so pointing it at `computer-use` means the stored token is not found
  and the next run asks the user to approve the session again. That is the honest
  consequence of moving it; reading the old location as well would keep the
  transition seamless at the cost of the alias rule above, and one prompt is
  cheaper than a compatibility path in a greenfield project.

- **The checkout directory and the GitHub remote are out of scope.** The working
  copy is `~/projects/github/desktop-use-rs`, and `Cargo.toml`'s `repository` field
  points at `https://github.com/felix/desktop-use-rs`. Renaming the remote is a
  GitHub operation, and moving the working directory would invalidate the open
  workspace and every path that refers to it, so neither is done here. The
  `repository` URL is therefore the one live place the old name remains, and it is
  left pointing at the remote that actually exists rather than at a URL that does
  not.

- **The library target is `computer_use`, not a hand-picked name.** Cargo derives it
  from the package name by replacing hyphens with underscores, so leaving it alone
  is what keeps `use computer_use::…` working from the binary, the examples, and the
  integration tests without an explicit `[lib] name`.

# Acceptance criteria

- `cargo build`, `cargo clippy --all-targets`, and `cargo test` all pass, with the
  binary built as `computer-use` and the library as `computer_use`.
- `initialize` reports `serverInfo.name` as `computer-use`.
- `tests/stdio.rs` spawns `CARGO_BIN_EXE_computer-use` and passes.
- No live artifact — the files under `src/`, `tests/`, and `examples/`,
  `README.md`, the current concept documents under `docs/`, or the front matter of
  the changelog — names the project `desktop-use`, except `Cargo.toml`'s
  `repository` field, which points at the un-renamed remote.
- The log filter is `COMPUTER_USE_LOG`, the state directory is `computer-use`, and
  the preview window, the capture thread, and the CLI all carry the new name.
- The change requests written before this one, their slugs, the changelog entries
  already written, and `docs/log.md` are unchanged in substance.

# Documentation

Updates `README.md` throughout — the title, the client configuration's server key
and command, the `COMPUTER_USE_LOG` example, the state-directory path, and the
prose that names the program. Updates the current concept documents: `docs/index.md`
and `docs/overview.md` (title and prose), `docs/requirements.md` (the human
operator's description), and `docs/dev/index.md`.

Records the rename in the [changelog](../changelog.md), since it is a shipped
change, and in the bundle [log](../../log.md), since it changes the identity the
documents are written under.

# Implementation plan

## Approach

A mechanical rename of the strings, then a build to let Cargo rewrite the lockfile
and confirm the derived target names. The three spellings that have to move
together are `desktop-use` (package, binary, server name, prose), `desktop_use`
(the Rust path), and `DESKTOP_USE` (the environment variable). `Cargo.lock` needs no
hand edit: Cargo rewrites the package entry on the next build.

The one place the rename must not be mechanical is the backlog: those files are
records, so they are left alone.

## Steps

- [x] Rename `desktop-use`, `desktop_use`, and `DESKTOP_USE` across `src/`,
  `tests/`, `examples/`, `README.md`, and the current documents under `docs/`.
- [x] Rename the package in `Cargo.toml`, leaving the `repository` URL pointing at
  the remote that exists.
- [x] Update the base64 expectation in `tools.rs`, whose literal spells the project
  name, to the encoding of the new one.
- [x] Build, which rewrites the package name in `Cargo.lock` and derives the
  `computer-use` binary and `computer_use` library targets.
- [x] Run clippy and the test suite.
- [x] Record the rename in the changelog and the bundle log.

# Verification

`cargo build`, `cargo clippy --all-targets`, and `cargo test` are all clean. The
suite passes 79 unit tests and 8 integration tests, including `initialize`, which
asserts `serverInfo.name` is `computer-use`, and the stdio tests, which spawn
`CARGO_BIN_EXE_computer-use`.

A repository-wide search for `desktop-use`, `desktop_use`, and `DESKTOP_USE`
returns matches only in the dated records and in `Cargo.toml`'s `repository` field,
exactly as the decisions describe.

# Acceptance criteria status

| Criterion | Status |
| --- | --- |
| Build, clippy, and tests pass, with the right target names | holds |
| `initialize` reports `serverInfo.name` as `computer-use` | holds: asserted by `tests/stdio.rs` |
| The stdio tests spawn `CARGO_BIN_EXE_computer-use` | holds |
| No live artifact names the project `desktop-use` but the `repository` URL | holds |
| The log filter, state directory, preview window, thread, and CLI carry the new name | holds |
| Pre-existing change requests, their slugs, earlier changelog entries, and the log are unchanged | holds |
