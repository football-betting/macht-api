# macht-api

[![macht-api-ci](https://github.com/football-betting/macht-api/actions/workflows/macht-api-ci.yml/badge.svg)](https://github.com/football-betting/macht-api/actions/workflows/macht-api-ci.yml)
[![codecov](https://codecov.io/gh/football-betting/macht-api/branch/master/graph/badge.svg)](https://codecov.io/gh/football-betting/macht-api)

Match-data importer for the office football-prediction game, built with Rust and
Tokio. It fetches fixtures and results from an external football data API and
writes them into the shared SQLite database. It runs as a one-shot job on a
schedule (e.g. a daily cron job or systemd timer), not as a long-running server.

## Architecture

Part of the `football-betting` workspace, alongside the `frontend` (Next.js) and
`betting-api` (read API). All three share a single SQLite database at
`../shared/db/database.db`.

- macht-api is the **only** writer of the `match` table. The frontend and
  `betting-api` only read it.
- The `match` schema is owned by the frontend (`frontend/db/schema.ts`,
  Drizzle) — keep the insert/update columns here in lockstep.

## Configuration

```bash
cp .env.dist .env
```

Then set:

| Variable        | Purpose                                                              |
|-----------------|----------------------------------------------------------------------|
| `X_AUTH_TOKEN`  | Auth token for the external football data API.                       |
| `API_URI`       | Endpoint to import from (e.g. `https://api.football-data.org/v4/competitions/WC/matches`). |
| `DB_PATH`       | Path to the shared SQLite file (e.g. `../shared/db/database.db`).     |

## Running

```bash
cargo run            # incremental import (current matchday window)
cargo run -- --full  # full import (all matches from the configured competition)
```

Schedule the incremental import to run regularly; use the full import for the
initial load or a backfill.

## Testing

```bash
cargo test
```

The tests are integration tests: they read `DB_PATH` and operate on a SQLite
database that must already contain the `match` table. They share one database
file and reuse match ids, so run them serially against a throwaway database —
never the shared one:

```bash
RUST_TEST_THREADS=1 DB_PATH=/tmp/macht-test.db cargo test
```

CI seeds that throwaway database (with the `match` schema, in WAL mode) before
running the suite. For coverage:

```bash
cargo tarpaulin --out Html
```
