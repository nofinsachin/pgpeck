---
inclusion: always
---

# Oxpecker: Project Structure

## Repository layout

```
oxpecker/
├── Cargo.toml
├── README.md
├── .kiro/
│   └── steering/          # Kiro steering files (this file lives here)
├── docs/
│   ├── requirements/      # UX and feature requirements
│   └── design/            # UX mockups and design notes
├── config/
│   ├── default.toml       # Documented default configuration
│   ├── keymap.default.toml
│   └── themes/
│       ├── dark.toml
│       └── light.toml
├── src/
│   ├── main.rs            # Entry point, CLI argument parsing, startup
│   ├── app/               # Application state and message handling
│   ├── ui/                # Rendering only: layout, panes, overlays, status bar
│   ├── editor/            # SQL editor buffer, cursor, selection, undo
│   ├── explorer/          # Find overlay, relationship view, context strip
│   ├── results/           # Result grid, record view, cell viewer
│   ├── db/                # Connection, query execution, cancellation
│   ├── catalog/           # Schema metadata loading and caching
│   ├── sql/               # Statement splitting, parsing, statement under cursor
│   ├── complete/          # Autocompletion sources and ranking
│   ├── keymap/            # Keymap loading, actions, conflict detection
│   ├── config/            # Config loading, validation, saving
│   ├── theme/             # Theme roles and color fallback
│   ├── history/           # Query history and snippets storage
│   ├── export/            # CSV, JSON, INSERT, markdown writers
│   ├── safety/            # Destructive-statement detection, confirmations, read-only mode
│   ├── secrets/           # Password sources and secret wrapper type
│   └── events/            # Input events, background task messages
├── tests/                 # Integration tests (some need PostgreSQL)
└── benches/               # Benchmarks, added with the features they measure
```

This layout is a starting point. Add a module when it has a clear purpose, and merge modules that stay small.

## Module responsibilities

- **`app`** owns the application state and the update function. It receives messages and returns new state plus any commands to run. It must not draw anything.
- **`ui`** reads state and draws it. It must not perform I/O or change state directly.
- **`editor`** is pure logic: text buffer, cursor, selection, undo, and bracket matching. It has no dependency on the terminal or the database, so it can be tested alone.
- **`sql`** is pure logic: statement splitting and the statement under the cursor (EDT-019 to EDT-022). It has no I/O.
- **`db`** is the only module that talks to PostgreSQL. Other modules receive results through messages.
- **`catalog`** caches schema metadata for the explorer and autocompletion. It refreshes after DDL (SCH-007).
- **`keymap`** maps keys to action names per scope. It detects conflicts and terminal-unsupported keys (KEY-003, KEY-004).
- **`safety`** decides whether a statement needs confirmation. Its decisions are pure functions, so they can be tested without a database.
- **`secrets`** is the only module that reads passwords. Other modules receive a redacted type.

## Naming conventions

- Modules and files: `snake_case`.
- Types: `UpperCamelCase`. Traits named by capability (for example, `Exporter`, `CompletionSource`).
- Action names: dotted lowercase, `area.verb_object`, for example `query.run_statement`, `explorer.open_relationships`, `results.copy_cell`. These names are stable public identifiers, since users put them in keymap files.
- Requirement IDs appear in comments only where a rule is non-obvious, for example `// SAF-003: UPDATE without WHERE needs confirmation`.
- Config keys: `snake_case` in TOML, grouped by section (`[editor]`, `[results]`, `[connections]`, `[keymap]`, `[theme]`, `[safety]`, `[history]`).

## Where new work goes

- **A new keybinding:** add the action in `keymap`, then the handler in `app`. Do not hardcode the key in `ui`.
- **A new visible element:** rendering in `ui` or the relevant feature module, with state in `app`.
- **A new export format:** a new file in `export`, implementing the shared writer trait.
- **A new destructive statement type:** `safety`, with a test in `tests/` or a unit test in `safety`.
- **A new config option:** `config` (validation and default), `config/default.toml` (documented), and the settings UI if CFG-003 applies.

## Configuration files

- `config/default.toml` is documented with a comment for each option. It is the reference for users.
- Project-local config (`.oxpecker.toml`) can override user config but cannot weaken safety settings for production connections (CFG-006).
- User config lives in the platform config directory. Its file mode is restricted to the owner (SEC-003).
- History and snippets live in the platform data directory, with owner-only permissions.

## Tests and docs placement

- Unit tests sit in the same file under `#[cfg(test)]`.
- Integration tests go in `tests/`. Tests that need PostgreSQL are named with a `pg_` prefix so they can be filtered out.
- Screen snapshots go in `tests/snapshots/`.
- Design mockups go in `docs/design/`. Update them when the layout changes.
- Keep the requirements IDs in `docs/requirements/` as the source of truth. Code and issues reference them.

## Git and workflow

- Work is tracked as GitHub issues, one per requirement or a small group of related requirements.
- Branch names: `feature/<short-name>` or `fix/<short-name>`.
- Commit messages reference the issue, for example `Add statement splitting (#12, EDT-019)`.
- Keep changes small enough to review in one sitting.
