# pgpeck

A fast, keyboard-driven terminal client for PostgreSQL, written in Rust.

<!-- Screenshots and a demo video go here. -->

## Status

Early development. You can connect, write and run SQL, read results, and find schema objects by
name. Not done yet: TLS (`ssl_mode` defaults to `disable`, and remote hosts refuse `prefer`).


## What it does

- **Connect** to a saved profile, a `postgres://` address, or a new set of details. pgpeck never
  stores a password.
- **Edit and run** SQL with syntax colors, completion, multiple buffers and bracket matching.
  `Ctrl+R` runs the statement under the cursor, or the selection.
- **Read results** in a grid with a header, column separators and row banding. A summary line
  (rows and time) sits under the results.
- **Browse long results.** A single `SELECT`, `WITH`, `VALUES` or `TABLE` query is paged: scroll
  past the edge of the loaded rows, or press `Home` or `End`, and pgpeck reads that part of the
  result from the server.
- **Go back** to what you ran. `Alt+H` lists your statements, newest first, and searches them.
- **Follow foreign keys.** Type `peck orders` in the Peck pane, or press `Alt+Z` on a table in the Find
  window, to see a diagram of what the table points to and what points to it. You can walk from
  table to table.
- **Find** tables, views, columns, functions, types and schemas by name with `Ctrl+P`.
- **Stay safe.** Risky statements wait for a typed `y` or `yes`. The session strip and the status
  bar flag open and failed transactions.
- **Watch the database.** The Trends pane shows connections, activity, size, health, deadlocks and
  the five largest tables.

## The screen

```
┌ Session ──────────────────────────────────────┐   ▄▄▄▄▄▄▄ (logo)      
│Session: name  Database: host:port, user, db   │┌ Context ────────────┐
└───────────────────────────────────────────────┘│Buffer, Statement    │
┌ Results ──────────────────────────────────────┐└─────────────────────┘
│column header │ column header                  │┌ Trends ─────────────┐
│rows ...                                       ││Connections, Activity│
│SELECT 2  ·  2 rows  ·  3 ms                   ││Size, Health, Deadlocks│
│                                               ││Largest tables       │
│                                               │└─────────────────────┘
│                                               │┌ Peck ───────────────┐
└───────────────────────────────────────────────┘│peck> orders         │
┌ Editor ───────────────────────────────────────┐│                     │
│select ...                                     ││                     │
│                                               │└─────────────────────┘
│                                               │
└───────────────────────────────────────────────┘
 hint bar (on narrow terminals)
 status bar
```

- **Session** (top left): the connection's name, where it points (host, user, database) and TLS.
  Production, open transactions and failures are flagged here.
- **Results** above the **Editor**, in the left column. A relationship diagram takes the place of
  the results while it is open.
- **Context**, **Trends** and **Peck** in the right column. The Peck pane is level with the editor.
  The logo sits above Context on tall terminals. Trends needs about 9 rows, and 13 to show all of
  its numbers and the five largest tables.

Copied text also goes to your system clipboard, through the terminal (OSC 52). Most modern
terminals allow this. If yours does not, check its clipboard setting, or paste with the terminal's
own paste key. Results are read-only. The full layout needs a terminal of at least 80x24.

## Build and run

```
cargo run
cargo run -- --check-config
```

## Try it against a database

Start a throwaway PostgreSQL, bound to localhost only:

```
docker run -d --name pgpeck-pg -e POSTGRES_USER=demo -e POSTGRES_PASSWORD=demo \
  -e POSTGRES_DB=demo -p 127.0.0.1:54329:5432 postgres:16
cargo run
```

Then in pgpeck:

1. Press `C`. Choose **New connection**, type the details (host `127.0.0.1`, port `54329`,
   database `demo`, user `demo`) and press `Enter`. Or paste
   `postgres://demo@127.0.0.1:54329/demo` in the box instead.
2. Type the password when asked (`demo`).
3. You land in the editor. Type `select now(), null as nothing;` and press `Ctrl+R`.

Remove the container with `docker rm -f pgpeck-pg`.

## Keys

`F1` lists every key. The main ones:

| Where | Key | Does |
|---|---|---|
| Context pane (where you start) | `C` | connect |
| | `F` | find a table, view, column, function, type or schema |
| | `R` | run the statement under the cursor |
| | `E` or `Enter` | go to the editor |
| | `N` | new buffer |
| | `Q` | quit |
| | `?` | help |
| Session strip (`Alt+4`) | `D` or `Enter` | change the database (reconnects) |
| | `U` | change the user (reconnects) |
| | `F5` | refresh the Trends numbers now |
| | `Z` | roll back an open or failed transaction |
| | `Esc` | back to the context pane |
| Any pane | `Ctrl+P` | open the Find window |
| | `Alt+H` | open the history window (`H` in the context pane) |
| | `Alt+Z` | roll back an open or failed transaction |
| | `F7` | show or hide Trends |
| | `F3` / `F4` | show or hide the right column / the results |
| | `Tab`, `F6` | next pane |
| Editor | `Ctrl+R` | run the statement under the cursor, or the selection |
| | `Ctrl+C` | cancel a running statement. Otherwise copy the selection |
| | `Esc` | back to the context pane |
| Results | arrows, `[` and `]` | move |
| Find window | `Enter` | put the object's name in the editor and copy it |
| | `Alt+T` `V` `M` `O` `F` `Y` `N` | show only tables, views, materialized views, foreign tables, functions, types or schemas. Press again to show all |
| | `Alt+C` | on a table or view: list only its columns |
| | `Alt+S` | on a table or view: add `SELECT * ... LIMIT 100` and run it |
| | `Alt+Z` | on a table or view: draw its relationships |
| Peck pane (`Alt+5`) | `Enter` | draw the selected table |
| | `Up` `Down` | choose among the matching tables |
| | `Esc` | back to the context pane |
| Relationship diagram | `Enter` | show the table on the other end of the selected key |
| | `Backspace` | back to the table you came from |
| | `S` | select sample rows of the selected table |
| | `Esc` | close |
| | `Esc` | close, changing nothing |
| Connect window | `Ctrl+E` | edit the selected saved connection |
| | `Ctrl+S` | save the typed address as a profile (no password) |

## Safety

Before a risky statement runs, pgpeck shows it with the connection name and waits for `y` or
`yes`. Enter alone does nothing. That applies to:

- any change on a production connection,
- `UPDATE` or `DELETE` without `WHERE`,
- `DROP` and `TRUNCATE`.

The session strip and the status bar show an open or failed transaction.

## Peck: relationships

Peck draws the foreign keys around a table. The **Peck** pane is in the right column, under the
trends and level with the editor. Press `Alt+5` (or `Tab` until it has focus), type a table or
view name, and press `Enter`. The word `peck` in front is optional, so `peck orders` and `orders`
do the same. While you type, the pane lists the tables and views that match. `Up` and `Down`
choose one. `Esc` leaves the pane.

The diagram takes the place of the results. The table sits in the middle. The tables that point
at it are on the left, and the tables it points at are on the right. Lines join the boxes, with
an arrow from the table that holds the foreign key to the table it refers to. Each box names the
columns of the key, holder first, for example `(order_id) → (id)`. A line under the diagram spells
out the selected key.

In the diagram, `Up` and `Down` move the selection through the right side first, then the left.
`Enter` makes the selected table the middle one, so you can walk through the schema, and
`Backspace` goes back. `S` selects sample rows of the selected table. `Esc` closes the diagram and
brings the results back. Running a statement closes it too. When a table has more neighbours than
fit, the diagram scrolls and says how many are out of view.

`Alt+Z` on a table in the Find window opens the same diagram. `peck schema` and `peck function`
are planned and say "not built yet". The keys come from the schema read when you connect. They
refresh after a statement that changes the schema.

## History

pgpeck remembers the statements you run, with the connection, the time taken and the row count.
`Alt+H` (or `H` in the context pane) opens the list, newest first. Type to search, `Enter` puts the
statement at the end of the editor, and nothing runs until you press `Ctrl+R`.

It never stores rows. The list is a private file (`history.tsv`, mode 0600) in
`$XDG_STATE_HOME/pgpeck`. Statements that mention a password are left out. Turn it off with
`history.enabled = false`, and change how many entries are kept with `history.max_entries`
(default 1000) in `config.toml`.

## Long results

A single read-only query (`SELECT`, `WITH`, `VALUES` or `TABLE`) is read through a server-side
cursor, so memory stays bounded and you can still reach every row. `results.row_limit` (default
10,000) is the size of the window held in memory. The line under the results shows
`rows 1–20 of 45310`. Scrolling to the edge of the window loads the next or previous one, and
`Home` and `End` jump to the first and last row of the whole result.

The server keeps the whole result in temporary storage until the next run in the same buffer,
which closes it. Set `results.paged = false` to keep only the first `row_limit` rows instead. Several
statements together, and anything that changes data, are not paged and keep only the first
`row_limit` rows. If you roll back a transaction that ran the query, its result can no longer be
paged, and pgpeck asks you to run it again.

## Trends

Trends shows the connected database only, in this order: connections, activity, size and cache
hit rate, health (oldest open transaction and uptime), deadlocks, then the five largest tables.
The connection count and limit are for the whole server, which every database on it shares. CPU
and memory are not shown, because PostgreSQL does not tell a client about them.

The numbers refresh every `session.refresh_seconds` (default 10, `0` turns it off) and after each
statement. They are read on your own connection, so never while a statement runs or a transaction
is open. Seeing other roles' sessions needs the `pg_read_all_stats` role.

## Connections and passwords

Naming a connection in the details form saves it as a profile in
`~/.config/pgpeck/connections.toml`, without a password. It is listed next time. pgpeck asks for
the password every time. To let it read one you already keep in `~/.pgpass` or `$PGPASSWORD`, see
`password_sources` in [`config/default.toml`](config/default.toml).

## Configuration

Files live in `$XDG_CONFIG_HOME/pgpeck` (usually `~/.config/pgpeck`):

- `config.toml`: see [`config/default.toml`](config/default.toml) for every option.
- `keymap.toml`: laid over [`config/keymap.default.toml`](config/keymap.default.toml).
- `themes/<name>.toml`: see [`config/themes`](config/themes). The dark theme follows
  postgresql.org's dark mode. A light theme is built in too.

`pgpeck --check-config` validates all three and exits non-zero on problems. Logs go to
`$XDG_STATE_HOME/pgpeck` (set `PGPECK_LOG=debug` for more).

## Development

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Tests named `pg_*` need a PostgreSQL and skip themselves without one. Point them at one with
`PGPECK_TEST_DATABASE_URL=postgres://user:password@127.0.0.1:5432/db cargo test`. CI runs them
against a `postgres:16` service.

Screen snapshots live in `tests/snapshots`. Rewrite them with `UPDATE_SNAPSHOTS=1 cargo test`.
Requirements are in [`.kiro/specs/requirements.md`](.kiro/specs/requirements.md).
