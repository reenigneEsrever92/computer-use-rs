# computer-use

`computer-use` is an MCP server that lets an AI agent observe and control a
Wayland Linux desktop. It captures the screen through the XDG Desktop Portal and
PipeWire, and injects pointer and keyboard input through the portal's
RemoteDesktop interface, exposing both as MCP tools over stdio.

## Contents

- [Overview](overview.md) — what the project is, who it is for, and what is in
  and out of scope.
- [Requirements](requirements.md) — the users, use cases, technology choices,
  and numbered requirements that define the first milestone.
- [Front matter schema](frontmatter.md) — the metadata every concept document
  and change request in this bundle carries.
- [Development](dev/index.md) — the change-driven workflow, the backlog, and the
  changelog.
