# oxpecker

A fast, keyboard-driven terminal client for PostgreSQL, written in Rust.

Status: early development. You can connect to a database, run statements and
read the results. Not done yet: TLS (remote hosts need `ssl_mode = "disable"`),
paged result browsing, the schema explorer, history and safety confirmations.

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
| Commands pane (top, where you start) | `C` | connect |
| | `R` | run the statement under the cursor |
| | `E` or `Enter` | go to the editor |
| | `N` | new buffer |
| | `Q` | quit |
| | `?` | help |
| Editor | `Ctrl+R` | run the statement under the cursor, or the selection |
| | `Esc` | back to the commands pane |
| | `Ctrl+C` | cancel a running statement |
| Results | `C` `R` `E` `Q` | as above, plus arrows, `[` and `]` to move |
| Connect window | type | search profiles, or paste a `postgres://` address |
| | `Ctrl+S` | save the typed address as a profile (no password) |

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
