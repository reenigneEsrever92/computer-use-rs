---
type: ChangeRequest
kind: feature
title: Scaffold the desktop-use server
description: Create the Rust binary, the MCP stdio skeleton, the backend traits, and the CLI including --preview.
state: done
priority: high
tags: [dev, scaffold, mcp]
owner: felix
verified: { by: agent, at: 2026-09-18T22:19:14+02:00 }
---

# Problem

The project is greenfield: there is no Rust crate, no MCP server, and no
structure in which capture and input backends can be added. Without a skeleton
that already answers MCP `initialize` and `tools/list`, and that already
separates the MCP layer from the backends, every later change request would have
to invent part of that structure — and the two decisions that are expensive to
retrofit, where the GUI event loop lives and where the backend abstraction sits,
would be made by accident instead of on purpose.

# Proposal

Create the `desktop-use` binary crate with the structure the later change
requests build on:

- A Cargo project, edition 2024, dual-licensed MIT OR Apache-2.0, with a
  `preview` cargo feature enabled by default.
- A stdio MCP server built on `rmcp`, behind a transport abstraction, answering
  `initialize` and `tools/list` with an empty tool set and shutting down cleanly
  when stdin closes.
- `CaptureBackend` and `InputBackend` traits, plus a registry that selects an
  implementation at startup. No implementation is provided yet: the traits are
  defined here so that the capture and input change requests implement them
  rather than invent them.
- CLI parsing for `--preview` and the options the later requests need
  (`--max-dimension`, image format), with `--preview` accepted and reporting
  that no capture backend is available yet.
- Error types with `thiserror`, surfaced to MCP clients, and `tracing` to
  stderr, with stdout reserved for the MCP channel.
- Startup checks for a graphical session: `DBUS_SESSION_BUS_ADDRESS`,
  `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, and the presence of `xdg-desktop-portal`
  and a backend, reported as actionable errors.
- The main-thread/worker-thread split that the preview window requires: when
  `--preview` is requested the GUI event loop owns the main thread and tokio runs
  on a worker thread; otherwise the main thread drives tokio.

# Decisions

- **The scaffold is its own change request, not part of the capture work.** It is
  independently verifiable — the server answers MCP requests with no desktop
  work at all — and it is where the CLI, error, and logging conventions are set.
- **`rmcp` rather than hand-rolled JSON-RPC.** See
  [requirements.md](../../requirements.md): the official SDK tracks the protocol,
  including tool-schema generation.
- **Backend traits defined now, implemented later.** R9's extensibility is
  cheapest to satisfy at the skeleton stage and expensive to retrofit once the
  tools call portal code directly.
- **`preview` as a default-on cargo feature.** Keeps the GUI dependency tree out
  of headless builds without introducing a second binary.
- **The main thread is reserved for the GUI when `--preview` is set.**
  `winit`/`eframe` require the event loop on the main thread; discovering that
  after the MCP runtime is wired up would mean restructuring the entry point.
- **Logging goes to stderr exclusively.** stdout carries MCP frames; a stray log
  line there corrupts the protocol.

# Acceptance criteria

- `cargo build` succeeds, and `cargo build --no-default-features` succeeds
  without the GUI dependencies.
- An MCP client can connect over stdio and complete `initialize`; `tools/list`
  returns an empty list; closing stdin shuts the process down with status 0.
- Running outside a graphical session produces an error naming the missing
  environment variable, not a panic and not a generic failure.
- A test asserts that nothing but MCP traffic is written to stdout.
- `--preview` is accepted and exits with a message that no capture backend is
  available yet, rather than opening a blank window.
- `CaptureBackend` and `InputBackend` exist and are documented, with no
  implementation yet.

# Documentation

No behaviour exists yet, so no existing document changes. This change adds a
`README.md` covering the runtime prerequisites — a graphical session with the
three environment variables, plus `xdg-desktop-portal` and a backend — and the
`--preview` flag, satisfying R13's documentation requirement. Later change
requests must keep it current as behaviour lands.

# Implementation plan

## Approach

One binary crate, `desktop-use`, in a `src/` module tree that keeps the MCP
layer, the CLI, and the backends apart:

- `main.rs` — parse CLI, initialise logging, check the session, dispatch. Owns
the main-thread/worker-thread split.
- `cli.rs` — `clap` derive types for the flags the milestone needs.
- `error.rs` — one `thiserror` enum, with the variants the startup checks and
the backends report through.
- `session.rs` — the graphical-session preflight checks that R10 requires.
- `backend/mod.rs` — the `CaptureBackend` and `InputBackend` traits, their
shared value types, and a registry that resolves an implementation at startup.
- `mcp.rs` — the `rmcp` server: `ServerHandler` plus a tool router that is empty
in this change and fills up in later ones.
- `preview.rs` — the `--preview` gate, which in this change only reports that no
capture backend exists yet.

Dependencies, and why each is needed now rather than later: `rmcp` with the
`transport-io` feature (the stdio transport is *not* in rmcp's default feature
set), `schemars` 1.x as a **direct** dependency (the tool macro's derive emits
bare `schemars::` paths, and rmcp requires 1.x — 0.8 would implement a different
`JsonSchema` trait and fail to compile), `tokio`, `serde`, `clap`, `thiserror`,
`tracing`, `tracing-subscriber`.

The `preview` cargo feature is declared now and gated on by default, so that
`--no-default-features` has something real to switch off and headless builds
have a supported path from the start. The GUI dependency it will gate arrives
with the preview window change request, which is the first change that needs it.

Trait shape: `CaptureBackend` is asynchronous and returns a logical-desktop-
coordinate `CaptureInfo` plus frames as owned RGB buffers, so the tool layer
never sees PipeWire or portal types. `InputBackend` is asynchronous and takes
logical desktop coordinates. Both are defined here, implemented from the capture
and input change requests on, and the registry resolves input availability so
`get_display_info` can report it (R8) without the MCP layer knowing why a backend
is absent.

## Steps

- [x] Create the crate, `Cargo.toml`, `.gitignore`, and the `preview` feature.
- [x] Add `cli.rs` with `--preview`, `--max-dimension`, and `--format`, and wire
  `main.rs` to parse it.
- [x] Add `error.rs` and `tracing` initialisation to stderr only.
- [x] Add `session.rs` with the preflight checks for `DBUS_SESSION_BUS_ADDRESS`,
  `XDG_RUNTIME_DIR`, and `WAYLAND_DISPLAY`, each reported by name. The portal
  check landed as a warning rather than an error: whether the portal answers on
  the session bus is the authoritative question, and the capture backend is
  where that is asked.
- [x] Define `CaptureBackend`, `InputBackend`, their value types, and the
  registry in `backend/`.
- [x] Add the `rmcp` stdio server with an empty tool router and a clean
  shutdown on stdin EOF.
- [x] Make `--preview` report that no capture backend is available yet.
- [x] Add a test asserting stdout carries nothing but MCP traffic.
- [x] Add a test asserting a missing environment variable yields a named error.
- [x] Write `README.md`: prerequisites, build, the `--preview` flag, and the
  `--no-default-features` build.

# Verification

Verified by `cargo build`, `cargo build --no-default-features`, and `cargo test`
on 2026-09-18: 8 unit tests and 6 stdio integration tests pass, and the build is
warning-free on both feature sets. There is no commit to link: the repository
exists but has nothing committed to it yet.

Two things the implementation settled that the request left open:

- The `preview` cargo feature is declared but gates no dependency yet. The GUI
  dependency it exists for arrives with the preview window change request, which
  is the first change that needs it, so `--no-default-features` is meaningful now
  and stays meaningful then.
- `#[tool_handler]` generates `get_info` from `Implementation::from_build_env()`,
  which reads *rmcp's* build environment and announced the server as `rmcp`
  1.8.0. The handler now overrides `get_info` to announce `desktop-use`.
- `rmcp` resolved to 1.8.0 rather than 1.7.0 under the `^1.7.0` requirement; the
  API used here is unchanged between them.
