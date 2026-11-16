# oxpecker

A fast, keyboard-driven terminal client for PostgreSQL, written in Rust.

Status: early development. You can connect to a database, run statements and
read the results. You can also find tables, views, columns, functions, types and schemas by name. Not done yet: TLS
(`ssl_mode` defaults to `disable`; remote hosts refuse `prefer`), paged result browsing, the
relationship view, the context strip and history.

## Build and run

```
cargo run
cargo run -- --check-config
```

## Try it against a database

Start a throwaway PostgreSQL (bound to localhost only):

```
docker run -d --name oxpecker-pg -e POSTGRES_USER=demo -e POSTGRES_PASSWORD=demo \
  -e POSTGRES_DB=demo -p 127.0.0.1:54329:5432 postgres:16
cargo run
```

Then in oxpecker:

1. Press `C`. Choose **New connection**, type the details (host `127.0.0.1`, port `54329`,
   database `demo`, user `demo`) and press `Enter`. Or paste
   `postgres://demo@127.0.0.1:54329/demo` in the box instead.
2. Type the password when asked (`demo`). Oxpecker never stores passwords. It asks every time.
3. You land in the editor. Type `select now(), null as nothing;` and press `Ctrl+R`.

The keys, all shown on screen:

| Where | Key | Does |
|---|---|---|
| Context pane (top left, where you start) | `C` | connect |
| | `F` | find a table, view, column, function, type or schema |
| | `R` | run the statement under the cursor |
| | `E` or `Enter` | go to the editor |
| | `N` | new buffer |
| | `Q` | quit |
| | `?` | help |
| Session pane (top right, `Alt+4` or `Tab`) | `D` or `Enter` | change the database (reconnects) |
| | `U` | change the user (reconnects) |
| | `F5` | refresh the numbers now |
| | `Esc` | back to the context pane |
| | `Z` | roll back an open or failed transaction |
| Top left | | the oxpecker, drawn when the terminal is 110 columns or wider |
| Any pane | `F7` | show or hide the Trends pane (the five largest tables, drawn when the terminal is 140 columns or wider) |
| Any pane | `Ctrl+P` | find a schema object (the Find window) |
| Find window | `Enter` | put the object's name in the editor, as listed, and copy it |
| | `Alt+T` `V` `M` `O` `F` `Y` `N` | show only tables, views, materialized views, foreign tables, functions, types or schemas. Press again to show all |
| | `Alt+C` | on a table or view: list only its columns. Press again to go back |
| | `Alt+Z` | relationship view (not built yet) |
| | `Alt+S` | on a table or view: add `SELECT * ... LIMIT 100` and run it |
| | `Esc` | close, changing nothing |
| Any pane | `Alt+Z` | roll back an open or failed transaction |
| Editor | `Ctrl+R` | run the statement under the cursor, or the selection |
| | `Esc` | back to the commands pane |
| | `Ctrl+C` | cancel a running statement |
| Results | `C` `R` `E` `Q` | as above, plus arrows, `[` and `]` to move |
| Connect window | `Ctrl+E` | edit the selected saved connection |
| | type | search profiles, or paste a `postgres://` address |
| | `Ctrl+S` | save the typed address as a profile (no password) |

Before a risky statement runs, oxpecker shows it with the connection name and waits for `y` or
`yes` (Enter alone does nothing). That applies to any change on a production connection, to
`UPDATE` or `DELETE` without `WHERE`, and to `DROP` or `TRUNCATE`. The status bar and the Session
pane show `TX OPEN` or `TX FAILED` while a transaction is open.

The Session pane shows the connected database only: its connections, active and idle-in-transaction
sessions, size, cache hit rate, oldest open transaction and deadlocks, next to the server's
connection count and limit, which every database on the server shares. CPU and memory are not
shown, because PostgreSQL does not tell a client about them. The numbers refresh every
`session.refresh_seconds` (default 10, 0 turns it off) and after each statement. They are read on
your own connection, so never while a statement runs or a transaction is open. Seeing other
roles' sessions needs the `pg_read_all_stats` role.

Naming a connection in the details form saves it as a profile in
`~/.config/oxpecker/connections.toml`, without a password. It is listed next time.
To let oxpecker read a password you already keep in `~/.pgpass` or `$PGPASSWORD`,
see `password_sources` in `config/default.toml`.

`F1` lists every key. Remove the container with `docker rm -f oxpecker-pg`.

## Configuration

Files live in `$XDG_CONFIG_HOME/oxpecker` (usually `~/.config/oxpecker`):

- `config.toml`: see [`config/default.toml`](config/default.toml) for every option.
- `keymap.toml`: laid over [`config/keymap.default.toml`](config/keymap.default.toml).
- `themes/<name>.toml`: see [`config/themes`](config/themes).

`oxpecker --check-config` validates all three and exits non-zero on problems.
Logs go to `$XDG_STATE_HOME/oxpecker` (set `OXPECKER_LOG=debug` for more).

## Development

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Tests named `pg_*` need a PostgreSQL and skip themselves without one. Point them at one with
`OXPECKER_TEST_DATABASE_URL=postgres://user:password@127.0.0.1:5432/db cargo test`.
CI runs them against a `postgres:16` service.

Screen snapshots live in `tests/snapshots`. Rewrite them with
`UPDATE_SNAPSHOTS=1 cargo test`. Requirements are in `.kiro/specs/requirements.md`.
