# Rust Api

[![macht-api-ci](https://github.com/football-betting/macht-api/actions/workflows/macht-api-ci.yml/badge.svg)](https://github.com/football-betting/macht-api/actions/workflows/macht-api-ci.yml)
[![codecov](https://codecov.io/gh/football-betting/macht-api/branch/master/graph/badge.svg)](https://codecov.io/gh/football-betting/macht-api)

### How to
- insert your key.json into the tmp directory
- execute `cp .env.dist .env`
- insert X_AUTH_TOKEN and API_URI into your .env
- run `cargo run` (daily import)
- run `cargo run -- --full` (full import)