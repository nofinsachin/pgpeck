---
inclusion: always
---

# Oxpecker: Technology Stack

## Language and toolchain

- **Language:** Rust, stable channel. Edition 2021 unless a newer edition is adopted deliberately.
- **Minimum supported Rust version (MSRV):** **Open.** Pin once the first release is planned.
- **Build:** Cargo. Single crate at the start. A workspace is added only when a crate boundary is needed.
- **Formatting:** `cargo fmt` must pass before commit.
- **Linting:** `cargo clippy --all-targets -- -D warnings` must pass before commit.
- **Tests:** `cargo test`. Unit tests live beside the code they test. Integration tests that need PostgreSQL go in `tests/` and are marked so they can be skipped without a database.

## Core dependencies

| Area | Choice | Status |
|---|---|---|
| Terminal rendering | `ratatui` | Proposed |
| Terminal input and backend | `crossterm` | Proposed |
| Async runtime | `tokio` | Proposed |
| PostgreSQL client | `tokio-postgres` or `sqlx` | **Open.** Decide before implementing the connection layer. |
| SQL parsing (statement splitting, highlighting) | `sqlparser-rs` or a tree-sitter SQL grammar | **Open.** Decide with EDT-022 (parser-based statement detection). |
| Fuzzy matching | `nucleo` or `fuzzy-matcher` | Proposed |
| Configuration | TOML via `serde` and `toml` | Proposed |
| Logging | `tracing` with a file subscriber | Proposed |
| Clipboard | `arboard` for system clipboard, plus OSC 52 output | Proposed |

Add a dependency only when it is needed by a requirement. Prefer well-maintained crates with few transitive dependencies.

## Architecture constraints

- **The UI thread never blocks on I/O.** Database calls, catalog loading, and exports run on async tasks and send results back through channels.
- **Rendering is separated from state.** Application state is updated by messages (events and results). Rendering reads state and draws it. This keeps the code testable without a terminal.
- **Keymaps and themes are data.** Both are loaded from files. Action names (for example, `query.run_statement`) are stable identifiers, and keys map to them.
- **Passwords are handled as secrets.** Use a secret wrapper type that does not implement `Debug` or `Display` in plain text. Never pass passwords through logs, history, or error messages.
- **Statement splitting uses the parser, not regex.** Dollar-quoted bodies and `BEGIN ... END` blocks must not be split incorrectly.

## Performance targets

- Keystroke-to-screen latency under 16 ms for editor typing (PRF-001).
- Startup to a usable screen under 300 ms, excluding connection time (PRF-002).
- Rendering a 1,000-row page does not block input (PRF-003).
- Memory is bounded for large result sets. Hold only loaded pages in memory (PRF-005).
- Redraws are incremental where practical (PRF-006).

Benchmarks are added when the relevant feature is built, not before.

## Terminal and platform

- Targets Linux and macOS terminals first.
- Must work over SSH and in tmux and screen.
- Minimum fully supported size is **Open** (see UX open questions). Below the minimum, fall back to a single pane with a resize message.
- Keyboard handling must account for terminals that cannot distinguish some modified keys (for example, `Ctrl+Enter`). Every core action needs a binding that works in all terminals.

## Testing approach

- Unit tests for statement splitting, keymap parsing and conflict detection, config validation, SQL generation for `SELECT ... LIMIT`, and export formatting.
- Snapshot tests for rendered screens at fixed terminal sizes, using `ratatui`'s test backend.
- Integration tests against a real PostgreSQL instance, run in CI through a container. These tests must not run by default on a developer machine without a database.
- Any change to safety logic (SAF-*) requires a test covering the confirmation path.

## Things to avoid

- Do not add `unsafe` code without a documented reason in a comment.
- Do not use `unwrap()` or `expect()` on values derived from user input, server responses, or files. Return errors and show them in the UI.
- Do not log SQL text at info level by default, since it can contain sensitive data.
- Do not add a dependency for a small helper that is straightforward to write.
- Do not hardcode key names in UI code. Use action names and look them up in the keymap.
