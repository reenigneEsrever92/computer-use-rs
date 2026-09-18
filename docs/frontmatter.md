---
type: Reference
title: Front matter schema
description: The metadata fields every concept document and change request in this bundle carries.
tags: [reference, conventions]
status: draft
---

# Front matter schema

Every document in this bundle except the reserved index and log files begins
with YAML front matter. `type` is the only required field; the rest are optional
unless noted.

## Concept documents

| Field | Required | Meaning |
| --- | --- | --- |
| `type` | yes | What kind of document this is. This bundle uses `Overview`, `Requirements`, `Reference`, `Changelog`, and `ChangeRequest`. |
| `title` | no | Human-readable name. Defaults to the filename. |
| `description` | no | One-line summary of what the document holds. |
| `tags` | no | Lowercase topic tags for grouping. `dev` marks documents in the development section. |
| `status` | no | The lifecycle of the document itself, not of the work it describes: `draft`, `stable`, or `superseded`. |

## Change requests

A change request is a concept document with `type: ChangeRequest`, so it also
carries `title`, `description`, and `tags`. It uses these extensions in place of
`status`:

| Field | Required | Meaning |
| --- | --- | --- |
| `kind` | yes | `feature`, `bug`, `improvement`, or `refactor`. |
| `state` | yes | Where the request is in the workflow: `proposed` → `planned` → `in-progress` → `done`, plus the terminal `rejected` and `superseded`. |
| `priority` | yes | `low`, `medium`, or `high`. |
| `owner` | yes | The actor accountable for the change. |

`status` and `state` are never both present: a change request uses `state`, and
every other concept uses `status`. A change request also carries the sections
the workflow expects — `# Problem`, `# Proposal`, `# Decisions`,
`# Acceptance criteria`, and a note of the documents it will touch.

## Reserved filenames

`index.md` and `log.md` are reserved and carry no front matter. `index.md` is
navigation; `log.md` records how the bundle itself changes over time. Reserved
filenames may appear at any level, so an index in a subdirectory is also
reserved.
