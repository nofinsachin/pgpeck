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

1. `Ctrl+O`, type `postgres://demo:demo@127.0.0.1:54329/demo`, `Enter`.
2. Type `select now(), null as nothing;` and press `F5` (also `Ctrl+Enter` or `Alt+Enter`).
3. `Alt+3` moves to the results, `Alt+2` back to the editor. `Ctrl+C` cancels a running statement.
4. In the picker, `Ctrl+S` saves the typed address as a profile, without its password.
   Saved profiles live in `~/.config/oxpecker/connections.toml` and are listed next time.
   Without a password in the address, oxpecker looks in the keychain, `~/.pgpass`
   and `$PGPASSWORD`, and asks if none has one.

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
