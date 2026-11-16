---
inclusion: always
---

# Oxpecker: Product

## What it is

Oxpecker is a fast, terminal-based PostgreSQL client written in Rust. It is a modern alternative to `psql` with a full-screen TUI: a SQL editor, a results grid, and an object explorer, all keyboard-driven.

## Target users

- Developers and DBAs who work in terminals, over SSH, or in tmux.
- Engineers who write SQL daily and want autocompletion and readable results without leaving the shell.
- Users who do not want to learn psql backslash commands.

## Product principles

- **Keyboard-first.** Every action is reachable from the keyboard. Mouse support is optional.
- **Modeless.** The editor always accepts text. No key changes what typing does. Focus moves between panes with keys that never produce characters.
- **Fast.** Typing latency, focus changes, and startup are measured against requirements (PRF-001 to PRF-003). Background work never freezes the UI.
- **Discoverable.** A hint bar, a help overlay, and a command palette (`Ctrl+P`) expose features without reading docs.
- **Safe by default.** Destructive statements require confirmation. Production connections are visually marked.
- **Configurable.** Keybindings, themes, and behavior live in files, and every single-key shortcut can be changed.
- **Efficient.** Prefer simple, practical features over flashy ones.

## Scope

In scope for the first release:

- Connecting to PostgreSQL and managing connection profiles.
- Writing and running SQL with statement-under-cursor execution.
- Browsing schema objects through the find overlay, the relationship view, and the context strip.
- Viewing results in grid and record views, and exporting them (CSV, JSON, INSERT, markdown).
- Query history, snippets, and transaction visibility.

Out of scope:

- psql backslash-command compatibility.
- Server administration (roles, backups, replication).
- Multi-user or networked collaboration.
- A plugin API.
- Windows support in the first release.

## Core UX model

The default screen has an editor, a context strip, a results pane, and a status bar. There is no permanent object tree.

- **Find overlay (`Ctrl+P`):** Fuzzy search across schemas, tables, columns, functions, types, actions, and history.
- **Relationship view:** Shows a table with its outgoing and incoming foreign keys. Following a relation moves the center, and a breadcrumb trail records the path.
- **Context strip:** Lists objects referenced by the statement under the cursor. It updates as the user types and never takes focus.
- **Miller-column browsing:** Optional layout for wide terminals.

## Requirements reference

Requirements are defined in the UX requirements document, using stable IDs (for example, `EDT-020`, `RUN-001`, `SAF-002`). Reference these IDs in issues, commits, and tests. Requirements prefixed `LAY-002`, `SCH-001`, and `SCH-002` are superseded by the explorer design and should not be implemented as written.

## Key decisions

- `Ctrl+Enter` runs the statement under the cursor. With a selection, it runs the selection.
- `Ctrl+C` cancels a running query. Otherwise it is unbound. **Open** (see the UX open questions).
- Passwords come from the OS keychain, `.pgpass`, an environment variable, or a prompt. They are never stored in plain text, logged, or shown on screen.
- Only the editor accepts printable characters. Single-letter shortcuts in other panes only work when those panes have focus.
