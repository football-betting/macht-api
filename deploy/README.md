# Deployment (systemd)

`macht-api` is a **oneshot** importer: it runs once (fetch + save), then exits.
A systemd **timer** triggers it on a schedule (this replaces the previous
PM2/cron trigger).

```bash
cargo build --release                       # builds target/release/rust-api
sudo cp deploy/macht-api.service deploy/macht-api.timer /etc/systemd/system/
# edit WorkingDirectory / EnvironmentFile / ExecStart paths to match your host
sudo systemctl daemon-reload
sudo systemctl enable --now macht-api.timer  # enable the timer, not the service
systemctl list-timers macht-api.timer        # confirm the schedule
journalctl -u macht-api -f                   # import logs
```

The importer runs every minute by default (`OnCalendar=*:0/1`). For a one-off
full import: `sudo systemctl start macht-api.service` (or run the binary with
`--full`). Config (`DB_PATH`, external API token, …) comes from the
`EnvironmentFile` (`.env`), never committed.

## Full-import safety timer (UTC-midnight rollover) — MA-010

The per-minute importer fetches only the current **UTC day**. A match that kicks
off late (e.g. 22:00 UTC) and is only marked `FINISHED` upstream after 00:00 UTC
is never re-fetched, so its `status` stays stuck at `IN_PLAY`/`PAUSED` (the
frontend then shows a finished game as "live"). A second timer runs the importer
with `--full` (no date filter) 3×/day to re-sync past days and finalize these
stragglers. A full import is a single upstream request, so 3×/day is negligible.

```bash
sudo cp deploy/macht-api-full.service deploy/macht-api-full.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now macht-api-full.timer
systemctl list-timers macht-api-full.timer
```

`OnCalendar=*-*-* 02,10,18:00:00 UTC` needs **systemd ≥ 252** (trailing-timezone
support). On **systemd < 252**, drop the `UTC` suffix and use the local-time
equivalents — e.g. on `Europe/Berlin`: `OnCalendar=*-*-* 04,12,20:00:00`
(= 02/10/18 UTC in CEST; in winter it drifts −1 h, which is harmless — the only
requirement is that one run lands after 00:00 UTC).

**Already installed & enabled (2026-06-14):** valantic (systemd 255, UTC form)
and fuhlingen (systemd 249, local `04,12,20:00:00` Europe/Berlin).
