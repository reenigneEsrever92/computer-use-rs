---
type: Overview
title: desktop-use
description: An MCP server that lets an AI agent observe and control a Wayland Linux desktop.
tags: [wayland, mcp, automation]
status: draft
---

# Overview

`desktop-use` is a Model Context Protocol (MCP) server that gives an AI agent
eyes and hands on a Linux desktop. An agent connects to it over stdio and calls
its tools to capture the screen — receiving the image as MCP image content — and
to move the pointer, click, scroll, type text, and send keyboard shortcuts.

The project exists because agents increasingly need to operate ordinary
graphical applications that expose no API: QA sweeps, filling in forms, GUI
regression checks, and "open this application and do X" tasks. The desktop is
the only interface those applications offer, and on Linux that desktop is
Wayland, which deliberately denies applications the global framebuffer access
and the input injection that X11 allowed. `desktop-use` works within Wayland's
rules rather than around them: the user consents once through the XDG Desktop
Portal, the compositor streams frames over PipeWire, and input is injected
through the portal's RemoteDesktop interface.

The project is greenfield and Wayland-first. It targets GNOME, KDE, and wlroots
compositors through their respective portal backends, but it is designed so that
other windowing systems and platforms can be added later: capture and input each
sit behind a backend trait rather than being wired directly into the MCP tools.

See the [requirements](requirements.md) for the users, use cases, technology
choices, and the numbered requirements of the first milestone, and the
[development section](dev/index.md) for how the work is tracked.
