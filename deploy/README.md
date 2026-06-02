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
