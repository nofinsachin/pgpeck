# Requirements Document

## Introduction

Oxpecker is a keyboard-driven, modeless, terminal-based PostgreSQL client written in Rust. It provides a full-screen SQL editor, a results grid, and a schema explorer built from three surfaces: a find overlay, a relationship view, and a context strip. There is no permanent object tree.

This document covers the first usable release. Requirement IDs from the UX requirements document (for example, `EDT-020`, `RUN-001`, `SAF-002`) are referenced where they apply. Where this document supersedes an earlier UX requirement, it says so.

## Glossary

- **Oxpecker**: The terminal application described by this document.
- **Editor**: The multi-line SQL text pane. It is the only pane that accepts printable characters.
- **Pane**: A top-level screen region: editor, context strip, results, or status bar.
- **Focus**: The single pane that receives key input at a given time.
- **Scope**: The keymap context in which a key binding applies: `global`, `editor`, `context`, `results`, `palette`, or `dialog`.
- **Action**: A named operation (for example, `query.run_statement`) that keys are bound to.
- **Connection Profile**: A saved set of connection parameters, excluding secrets.
- **Statement Under Cursor**: The SQL statement that contains the editor cursor, determined by the SQL parser.
- **Context Strip**: The pane listing schema objects referenced by the statement under cursor. It never takes focus.
- **Find Overlay**: The modal-looking but non-mode search popup opened by `Ctrl+P` that finds schema objects, actions, and history.
- **Relationship View**: A pane showing one table with its outgoing and incoming foreign-key relations.
- **Catalog**: Cached schema metadata (schemas, relations, columns, functions, types, constraints) read from PostgreSQL.
- **Staged Edit**: A pending change to table data in the results grid that has not been applied.
- **Production Connection**: A connection profile marked `production = true`.
- **Destructive Statement**: A statement that drops, truncates, or modifies rows without a `WHERE` clause.

## Requirements

### Requirement 1: Connection profiles

**User Story:** As a database user, I want to save and reuse connection profiles, so that I can connect to my databases quickly without retyping parameters.

#### Acceptance Criteria

1. WHEN the user creates a profile THEN the system SHALL store host, port, database, user, SSL mode, an optional label, an optional color, and a production flag.
2. WHEN the user edits, duplicates, or deletes a profile THEN the system SHALL apply the change and persist it to the user configuration.
3. WHEN the user opens the connection picker THEN the system SHALL list profiles with fuzzy search and show recently used profiles first.
4. WHEN the user opens a profile from the picker THEN the system SHALL connect to that profile and make it the active connection for the focused buffer.
5. WHEN the user supplies a connection URI THEN the system SHALL connect without saving a profile.
6. IF a profile is marked as a production connection THEN the system SHALL display its label with a `PROD` marker and a distinct status bar color.

### Requirement 2: Credential handling

**User Story:** As a security-conscious user, I want passwords handled as secrets, so that they are never shown, logged, or stored in plain text.

#### Acceptance Criteria

1. WHEN a connection needs a password THEN the system SHALL read it from the OS keychain, a `.pgpass` file, an environment variable, or an interactive prompt, in that configurable order.
2. WHILE a password is held in memory THEN the system SHALL represent it with a secret type that does not expose its value through `Debug` or `Display`.
3. THE system SHALL NOT write passwords or tokens to logs, query history, configuration files, crash reports, or the clipboard.
4. IF a connection error occurs THEN the system SHALL describe the cause and the next step in plain language without including the password.
5. WHEN the user copies a connection string that contains credentials THEN the system SHALL warn the user before copying.
6. THE system SHALL create configuration, history, snippet, and crash-recovery files with owner-only file permissions.

### Requirement 3: Connection state and identity

**User Story:** As a user working across several databases, I want to always see which database I am connected to and in what state, so that I do not run statements against the wrong target.

#### Acceptance Criteria

1. THE status bar SHALL show the connection label, database, role, and transaction state at all times while a connection is active.
2. WHEN the connection state changes THEN the system SHALL update the status bar to one of: connecting, connected, reconnecting, or failed.
3. IF the connection drops THEN the system SHALL show a notice, offer reconnection, and keep unsent editor content.
4. WHERE TLS is in use, the system SHALL display the TLS state in the status bar.
5. WHERE a connection has a configured color, the system SHALL show that color in the status bar and the context strip header.

### Requirement 4: Keyboard-only interaction and focus

**User Story:** As a keyboard-first user, I want to reach every action from the keyboard without changing modes, so that my typing always goes where I expect.

#### Acceptance Criteria

1. THE system SHALL provide a keyboard route to every action. Mouse input SHALL NOT be required for any action.
2. THE system SHALL have exactly one focused pane at a time, and the status bar SHALL name the focused pane.
3. WHEN the editor has focus and the user types a printable character THEN the system SHALL insert that character into the buffer.
4. WHILE the editor has focus, no pane-scoped shortcut SHALL consume a printable character.
5. WHEN the user presses a focus key (defaults `Ctrl+1`, `Ctrl+2`, `Ctrl+3`) THEN the system SHALL move focus to the sidebar-equivalent context strip, the editor, or the results pane respectively.
6. WHEN the user presses `Tab` or `Shift+Tab` THEN the system SHALL cycle focus through visible panes, unless the editor has focus and tab-indent is enabled.
7. THE system SHALL always provide a keyboard route out of the editor that does not depend on `Tab`.
8. WHEN a pane-scoped single-key shortcut is pressed while a different pane has focus THEN the system SHALL ignore it.

### Requirement 5: Configurable keymap

**User Story:** As a user with established key habits, I want every single-key shortcut to be configurable, so that I can avoid conflicts with my terminal or OS.

#### Acceptance Criteria

1. THE system SHALL define all key bindings in a keymap file, and a default keymap SHALL ship with the application.
2. WHEN the keymap is loaded THEN the system SHALL refer to each binding by its action name.
3. IF two actions in the same scope share a key THEN the system SHALL report the conflict at load time and refuse the duplicate binding.
4. IF a configured key cannot be delivered by the terminal THEN the system SHALL warn the user and use the documented fallback key, and the status bar SHALL indicate the fallback.
5. THE system SHALL provide at least one binding for each core action that works in terminals without advanced keyboard protocols.
6. WHEN the user runs `oxpecker --check-config` THEN the system SHALL validate the configuration and keymap without starting the interface.
7. WHERE the user requests the keyboard reference THEN the system SHALL display all bindings for the focused scope and the global scope.

### Requirement 6: SQL editor

**User Story:** As a SQL author, I want an editor that understands PostgreSQL syntax, so that I can write and edit queries efficiently.

#### Acceptance Criteria

1. THE editor SHALL support multi-line editing, cursor movement, selection, cut, copy, paste, undo, and redo.
2. THE editor SHALL highlight PostgreSQL keywords, strings, comments, quoted identifiers, and dollar-quoted bodies.
3. WHEN the user types a prefix THEN the system SHALL show completion suggestions for keywords, schemas, relations, columns, functions, and types drawn from the catalog.
4. WHEN a completion popup is open and the user presses `Enter` THEN the system SHALL accept the highlighted suggestion.
5. WHEN no completion popup is open and the user presses `Enter` THEN the system SHALL insert a newline.
6. WHEN the user presses `Esc` with a completion popup open THEN the system SHALL dismiss the popup without changing the buffer.
7. THE editor SHALL auto-indent and match brackets and quotes, and the user SHALL be able to disable auto-pairing.
8. THE editor SHALL show line and column numbers, and the status bar SHALL show the current cursor position.
9. WHERE the user requests formatting THEN the system SHALL format the statement under cursor or the whole buffer, and formatting SHALL NOT run automatically.
10. THE editor SHALL support multiple buffers, each with its own unsaved state and connection.

### Requirement 7: Statement detection

**User Story:** As a user who writes functions and multi-statement scripts, I want the editor to find statement boundaries correctly, so that the statement I run is the one I intended.

#### Acceptance Criteria

1. THE system SHALL determine statement boundaries using a SQL parser, not regular expressions.
2. WHEN a semicolon appears inside a string literal, quoted identifier, comment, dollar-quoted body, or `BEGIN ... END` block THEN the system SHALL NOT treat it as a statement separator.
3. WHEN the cursor is within a statement THEN the system SHALL identify that statement as the Statement Under Cursor.
4. WHEN the cursor is on whitespace between two statements THEN the system SHALL select the preceding statement, or the following one if no preceding statement exists.
5. WHILE `split_on_blank_line` is enabled and the buffer contains a blank line THEN the system SHALL treat the blank line as a statement separator.
6. WHEN a statement is about to run THEN the system SHALL highlight it in the editor.

### Requirement 8: Running queries

**User Story:** As a user running queries, I want execution that never blocks my editing, so that I can keep working while a query runs.

#### Acceptance Criteria

1. WHEN the user presses `Ctrl+Enter` with no selection THEN the system SHALL run the Statement Under Cursor.
2. WHEN the user presses `Ctrl+Enter` with a selection THEN the system SHALL run the selected text.
3. WHILE a query runs THEN the system SHALL keep the editor fully editable.
4. WHILE a query runs THEN the status bar and results pane SHALL show elapsed time and a progress indicator.
5. WHEN the user requests cancellation THEN the system SHALL send a PostgreSQL cancel request and report whether the cancellation succeeded.
6. IF a second query is requested on a buffer whose connection is busy THEN the system SHALL ask the user to queue, replace, or cancel the running query.
7. WHEN a run contains multiple statements THEN the system SHALL produce one result tab per statement, each labeled with its statement number and command tag.
8. WHEN a non-query statement completes THEN the system SHALL show its command tag (for example, `INSERT 0 5`).
9. WHEN a query completes THEN the system SHALL show its execution time with the result and record the duration in history.
10. WHERE stop-on-error is enabled THEN the system SHALL stop a multi-statement run at the first error.
11. WHEN the server sends notices or warnings THEN the system SHALL show them in a messages area and not in the results grid.
12. WHERE a query runs longer than a configurable threshold (default 2 seconds) and the terminal is unfocused THEN the system SHALL notify the user when it finishes.
13. WHEN the user runs a statement with `EXPLAIN` or `EXPLAIN ANALYZE` through a binding THEN the system SHALL display the plan in a dedicated view.

### Requirement 9: Schema explorer: find overlay

**User Story:** As a user who knows roughly what I am looking for, I want to find any schema object by name, so that I can open it without browsing a tree.

#### Acceptance Criteria

1. WHEN the user presses `Ctrl+P` THEN the system SHALL open the Find Overlay with the search box focused.
2. WHEN the user types in the Find Overlay THEN the system SHALL show fuzzy-matched results across schemas, relations, columns, functions, types, actions, connections, snippets, and history.
3. THE Find Overlay SHALL show each result with its kind and schema-qualified path, so that same-named objects in different schemas remain distinguishable.
4. THE Find Overlay SHALL group results by kind and show the keybinding for each action that has one.
5. WHEN the user presses `Enter` on a relation in the Find Overlay THEN the system SHALL open the Relationship View for that relation.
6. WHEN the user presses `Alt+D` on an object in the Find Overlay THEN the system SHALL show its definition in the results pane as read-only.
7. WHEN the user presses `Alt+S` on a table in the Find Overlay THEN the system SHALL insert `SELECT * FROM <table> LIMIT n` into the editor and run it, where `n` is the configured default (100).
8. WHEN the user presses `Esc` THEN the system SHALL close the Find Overlay without changing the editor or the focus.
9. THE Find Overlay SHALL NOT use single-letter shortcuts that conflict with its search input.

### Requirement 10: Context strip

**User Story:** As a user writing a query, I want to see the objects my statement uses without opening anything, so that I can check names and types while I type.

#### Acceptance Criteria

1. THE context strip SHALL list the relations referenced by the Statement Under Cursor, with their columns and types.
2. WHEN the Statement Under Cursor changes THEN the context strip SHALL update without the user pressing a key.
3. THE context strip SHALL NOT take focus when it updates. Typing SHALL always remain in the editor.
4. WHERE a relation has foreign-key relations THEN the context strip SHALL show the number of relations next to it.
5. WHEN the user presses `F3` THEN the system SHALL show or hide the context strip.
6. WHEN the user moves focus to the context strip with `Ctrl+1` and presses `Enter` on an item THEN the system SHALL open the Relationship View for that item.
7. IF the terminal is narrower than the minimum width THEN the system SHALL collapse the context strip to a single line.

### Requirement 11: Relationship view

**User Story:** As a user trying to understand how data connects, I want to see a table's foreign-key relations in both directions, so that I can navigate between related tables.

#### Acceptance Criteria

1. WHEN the Relationship View opens for a table THEN the system SHALL show the table name, schema, and estimated row count from catalog statistics, marked as an estimate.
2. THE Relationship View SHALL show outgoing foreign keys on the left and incoming foreign keys on the right.
3. WHEN the user presses `Enter` on a related table THEN the system SHALL make that table the center of the view and append it to a breadcrumb trail.
4. WHEN the user presses `Esc` or `Shift+Tab` in the Relationship View THEN the system SHALL return to the previous center table in the breadcrumb trail.
5. WHEN the user presses `Tab` in the Relationship View THEN the system SHALL show the detailed view with columns, types, nullability, defaults, indexes, and constraints.
6. WHEN the user presses `d` in the Relationship View THEN the system SHALL show the table DDL read-only in the results pane.
7. WHEN the user presses `s` in the Relationship View THEN the system SHALL run the sample-rows query described in Requirement 9, criterion 7.
8. IF the terminal is narrower than 100 columns THEN the system SHALL stack the relations vertically.

### Requirement 12: Results grid and record view

**User Story:** As a user reading query output, I want a readable, keyboard-navigable grid that can also show one row at a time, so that I can inspect both wide and narrow results.

#### Acceptance Criteria

1. THE results pane SHALL display rows in a column-aligned grid with a header row that remains visible while scrolling.
2. THE results pane SHALL show column names and types in the header, and truncate long names with a visible indicator.
3. THE results pane SHALL display NULL values in a styled, labeled form that is distinct from an empty string.
4. WHEN the user presses page up, page down, first-row, or last-row keys THEN the system SHALL move through the result accordingly.
5. WHEN the user presses the record-view key THEN the system SHALL show the current row as name-value pairs with wrapped long values, and the next and previous keys SHALL move between rows.
6. WHILE the result is streaming THEN the results header SHALL show the number of rows loaded and whether more rows exist.
7. THE results header SHALL show the row count and query time.
8. WHEN the user sorts or hides a column THEN the system SHALL apply the change client-side without re-running the query.
9. IF the row limit is reached THEN the system SHALL display a message stating that the limit was reached.
10. THE system SHALL NOT hold the full result set in memory when it exceeds a configured limit. Only loaded pages and the record-view window are kept.

### Requirement 13: Cell and value inspection

**User Story:** As a user with complex values, I want to inspect full cell contents, so that I can read JSON, arrays, and binary data properly.

#### Acceptance Criteria

1. WHEN the user presses `Enter` on a cell THEN the system SHALL open a value viewer with the full value, scrollable and wrapped.
2. WHEN the value is `json` or `jsonb` THEN the viewer SHALL pretty-print it with collapsible nodes.
3. WHEN the value is an array THEN the viewer SHALL show one element per line.
4. WHEN the value is `bytea` THEN the viewer SHALL show a hexadecimal view and a size summary, and SHALL load no more than the configured maximum into memory.
5. WHEN the user presses the copy key in the viewer THEN the system SHALL copy the full value to the clipboard and report it in the status bar.
6. WHEN the user chooses to save a value THEN the system SHALL write it to a file the user selects.

### Requirement 14: Export and copy

**User Story:** As a user who needs data outside the client, I want to export results in common formats, so that I can use them in other tools.

#### Acceptance Criteria

1. THE system SHALL export results as CSV, JSON (array of objects), INSERT statements, or a markdown table.
2. WHEN the user exports CSV THEN the system SHALL allow setting the delimiter, quoting, header inclusion, and NULL representation.
3. WHEN the user exports a result THEN the system SHALL include the full result set, not only the loaded rows.
4. IF the result is still streaming when export is requested THEN the system SHALL wait for completion or warn that the export is incomplete.
5. WHEN the target file exists THEN the system SHALL ask for confirmation before overwriting it.
6. WHILE an export runs THEN the system SHALL show progress in the status bar and keep the editor usable.
7. WHEN the user copies rows, a column, or the whole result to the clipboard THEN the system SHALL use the same formats as export.
8. IF neither the system clipboard nor OSC 52 is available THEN the system SHALL say so and point to file export.

### Requirement 15: Safety and confirmations

**User Story:** As a user working against real data, I want destructive actions to require explicit confirmation, so that I do not lose data by accident.

#### Acceptance Criteria

1. WHEN a statement that is not read-only is about to run on a production connection THEN the system SHALL require confirmation before running it.
2. WHEN an `UPDATE` or `DELETE` without a `WHERE` clause is about to run THEN the system SHALL warn and require confirmation, unless the user disabled this check for that connection.
3. WHEN a `DROP` or `TRUNCATE` statement is about to run THEN the system SHALL require confirmation, unless the user disabled this check for that connection.
4. THE confirmation dialog SHALL show the statement text and the connection name.
5. THE confirmation dialog SHALL accept `y` or `yes` as confirmation and SHALL NOT activate on `Enter` alone.
6. WHERE a session is set to read-only THEN the system SHALL reject write statements and display a clear message.
7. WHEN the user edits table rows in the results grid THEN the system SHALL stage the changes and show a summary of pending inserts, updates, and deletes.
8. WHEN the user applies staged edits THEN the system SHALL apply them in a single transaction and roll back all of them if any fails.
9. WHEN the user discards staged edits THEN the system SHALL discard all of them.
10. WHEN the user closes the application with staged edits or an open transaction THEN the system SHALL offer commit, roll back, or cancel the exit.

### Requirement 16: Transactions

**User Story:** As a user who uses explicit transactions, I want the transaction state visible at all times, so that I do not leave work uncommitted.

#### Acceptance Criteria

1. THE status bar SHALL show the transaction state: none, in transaction, or failed transaction.
2. WHEN a transaction is left open after a query finishes THEN the system SHALL flag it until the transaction ends.
3. WHEN a transaction is in the failed state THEN the system SHALL offer rollback.
4. THE system SHALL provide `BEGIN`, `COMMIT`, and `ROLLBACK` as palette actions, and SHALL also execute them when typed in the editor.
5. WHERE autocommit is disabled for a connection THEN the system SHALL treat statements as part of an open transaction until the user commits or rolls back.

### Requirement 17: Query history and snippets

**User Story:** As a user who repeats queries, I want a searchable history and reusable snippets, so that I can find and reuse past work.

#### Acceptance Criteria

1. WHEN a statement is executed THEN the system SHALL record its timestamp, connection, duration, row count or error, and originating buffer in history.
2. WHEN the user presses `Ctrl+R` THEN the system SHALL open a fuzzy-searchable history that can be filtered by connection.
3. WHEN the user selects a history entry THEN the system SHALL insert it at the cursor or into a new buffer, according to configuration.
4. THE system SHALL NOT record statements that match the secret-exclusion patterns, including `PASSWORD` in `CREATE USER` or `ALTER ROLE`.
5. WHEN the user adds an exclusion pattern THEN the system SHALL apply it to future history entries.
6. THE user SHALL be able to clear history for one connection or for all connections.
7. WHEN history exceeds the configured size or retention period THEN the system SHALL remove the oldest entries.
8. WHEN the user saves a snippet THEN the system SHALL store its name, optional description, and optional connection scope.
9. THE user SHALL be able to insert, edit, and delete snippets from the Find Overlay.

### Requirement 18: Status bar and notifications

**User Story:** As a user, I want the status bar and hint bar to tell me what is happening and what keys are available, so that I do not need to read documentation.

#### Acceptance Criteria

1. THE status bar SHALL show, from left to right: connection label, database, role, transaction state, focused pane, and current activity such as a running query or row count.
2. THE status bar SHALL show the execution time of the most recent query.
3. THE hint bar SHALL show the keys available in the focused pane and update when focus changes.
4. WHEN a notice is produced (for example, "Clipboard copied") THEN the system SHALL show it in the status bar for a few seconds and record it in a notifications log.
5. WHERE configured THEN the system SHALL allow the hint bar to be hidden or reduced to one line.
6. WHEN the user presses `F1` or `?` in a non-editor pane THEN the system SHALL show a help overlay listing bindings for the focused scope.

### Requirement 19: Errors

**User Story:** As a user who sees server errors, I want clear error details and their position in my SQL, so that I can fix mistakes quickly.

#### Acceptance Criteria

1. WHEN the server returns an error THEN the system SHALL show the SQLSTATE code, message, detail, and hint, where provided.
2. WHEN the error includes a position THEN the system SHALL mark the line and column in the editor with a gutter indicator and highlight the token.
3. THE system SHALL keep a scrollable log of notices, warnings, and errors for the session.
4. WHEN the user copies an error THEN the system SHALL copy it as plain text.

### Requirement 20: Layout and terminal behavior

**User Story:** As a user who works over SSH and in tmux, I want the interface to adapt to my terminal, so that it stays usable in a range of environments.

#### Acceptance Criteria

1. THE default layout SHALL contain the editor, the context strip, the results pane, and the status bar.
2. THE results pane SHALL be showable or hideable, and its height SHALL be adjustable by keybinding.
3. WHEN the user maximizes a pane THEN the system SHALL fill the screen with it, and restoring SHALL return the previous layout.
4. WHEN the terminal is resized THEN the system SHALL redraw the layout without corrupting it.
5. IF the terminal is below the minimum supported size THEN the system SHALL show one pane at a time and a message describing the required size.
6. WHEN the application starts THEN the system SHALL restore the last layout unless the user has disabled this.
7. THE application SHALL run over SSH and inside tmux and screen without requiring a graphical environment.
8. WHERE the terminal supports 16, 256, or truecolor modes THEN the system SHALL use the best supported mode and fall back gracefully.
9. THE system SHALL NOT convey focus, errors, or NULL status by color alone.
10. WHEN a bracketed paste occurs THEN the system SHALL insert the pasted text without executing statements.

### Requirement 21: Configuration and themes

**User Story:** As a user who wants to customize the tool, I want settings and themes in readable files, so that I can change behavior without rebuilding the application.

#### Acceptance Criteria

1. THE system SHALL read behavior settings from a single human-readable configuration file, documented with a comment for each option.
2. IF the configuration contains an invalid option THEN the system SHALL report the file, line, and option at startup and fall back to the default for that option.
3. WHEN the user changes a setting in the interface and saves THEN the system SHALL write it to the configuration file.
4. WHERE a project-local configuration file exists THEN the system SHALL apply its settings over the user configuration, except that it SHALL NOT weaken safety settings for production connections.
5. THE system SHALL load themes from files, and a theme SHALL define color roles such as `focus`, `error`, `null`, `dimmed`, `keyword`, `string`, and `status_prod`.
6. WHEN the user changes the theme from the Find Overlay THEN the system SHALL apply the new theme without restarting.

### Requirement 22: Performance

**User Story:** As a user who types quickly and works with large tables, I want the interface to stay responsive, so that work is never interrupted by slow operations.

#### Acceptance Criteria

1. WHEN the user types in the editor THEN the system SHALL show the character within 16 ms on typical hardware.
2. WHEN the application starts THEN the system SHALL show a usable screen within 300 ms on typical hardware, excluding connection time.
3. WHILE a result page of 1,000 rows is rendered THEN the system SHALL continue to accept input.
4. WHILE catalog loading or completion requests are in progress THEN the system SHALL continue to accept input.
5. WHEN the user changes focus or opens a view THEN the system SHALL respond within 50 ms on typical hardware, even while a query runs.

### Requirement 23: Quality and maintainability

**User Story:** As a maintainer, I want rules that keep the code testable and safe, so that the project stays healthy as it grows.

#### Acceptance Criteria

1. THE build SHALL pass `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` before a change is merged.
2. THE system SHALL NOT call `unwrap()` or `expect()` on values derived from user input, server responses, or files.
3. THE system SHALL NOT log SQL text at info level by default.
4. WHEN a change affects safety logic THEN the change SHALL include a test covering the confirmation path.
5. WHERE a feature needs a live PostgreSQL instance THEN its tests SHALL be marked so they can be skipped without a database.

## Out of Scope

- psql backslash-command compatibility.
- Server administration (roles, backups, replication).
- Multi-user or networked collaboration.
- A plugin API.
- Windows support.
- Multi-cursor editing, Parquet or Excel export, and inline type hints (deferred to later releases).

## Open Questions

- Should `Ctrl+C` cancel a running query or copy text? Current draft: cancel only while a query runs, otherwise unbound.
- Should the results pane be editable by default for table views, or read-only until the user opts in?
- What is the minimum fully supported terminal size?
- Should history store results or only statements and metadata?
- Which keys does macOS `Option` conflict with in common terminal emulators?
