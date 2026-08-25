# WAN telemetry collector — operator guide

Server side of the opt-in alpha diagnostics: an HTTPS ingest endpoint for
alpha testers on the public internet, plus a read-only review portal for the
operator on the tailnet. The anonymized payload is produced by
[`src/diagnostics.rs`](../../src/diagnostics.rs); the user-facing privacy
statement that ships with alpha builds is [`PRIVACY.md`](PRIVACY.md).

Components (this directory):

| File | Role |
|------|------|
| `app.py` | FastAPI ingest — `POST /v1/diagnostics`, `GET /health` |
| `portal.py` | Read-only report viewer for the operator, port 8788 |
| `db.py` | SQLite access layer (WAL mode) |
| [`deploy/deploy.sh`](../../deploy/deploy.sh) | Installs the services and the Caddy site on the VPS |

## Architecture

```
             public internet (WAN)                      tailnet (Tailscale only)
 ┌───────────────┐  HTTPS  ┌──────────────────────────┐         ┌─────────────┐
 │ Alpha client  │ ──────► │ Caddy :443               │         │ Operator    │
 │ opt-in POST   │         │ TLS for $LEGION_WAN_     │         │ browser     │
 │ (curl, ≤15 s) │         │ DOMAIN — fills the       │         └──────┬──────┘
 └───────────────┘         │ nginx-equivalent reverse │                │ role
                           │ proxy from the old plan  │                ▼
                           └────────────┬─────────────┘     https://127.0.0.1:8788
                                        │ proxy to loopback        │
                                        ▼                          ▼
                        ┌──────────────────────────────┐   ┌──────────────────────┐
                        │ app.py   127.0.0.1:8787      │   │ portal.py   :8788    │
                        │ · requires shared-key header │   │ no auth — bound to   │
                        │   (401 otherwise)            │   │ the tailnet; must    │
                        │ · rate limit 30/min/IP (429) │   │ never be exposed     │
                        │ · body cap 256 KiB           │   │ publicly             │
                        └───────────────┬──────────────┘   └──────────┬───────────┘
                                        ▼                             │ reads
                        ┌──────────────────────────────────────────────┐   │
                        │ SQLite in WAL mode — $LEGION_TELEMETRY_DB    │◄──┘
                        │ 1 row = UTC timestamp + raw anonymized JSON; │
                        │ rows older than 90 days pruned hourly        │
                        └──────────────────────────────────────────────┘
```

- **Ingest requires the shared key.** Every `POST /v1/diagnostics` must carry
  `X-Legion-Telemetry-Key: <secret>` matching `$LEGION_TELEMETRY_KEY`;
  mismatching or missing headers get HTTP 401 (constant-time compare).
- **The portal has no login** because it is reachable only through Tailscale
  (`127.0.0.1:8788`) — the tailnet is the authentication boundary. Never
  forward or reverse-proxy port 8788 to the public internet.
- Client IP is seen only while a request is in flight (in-memory sliding
  window for rate limiting) and is never persisted.

## Fixed contract

Endpoints and ports:

| Endpoint | Where | Notes |
|----------|-------|-------|
| `POST /v1/diagnostics` | Caddy :443 → ingest 127.0.0.1:8787 | JSON report, `schema_version` must be `1`; body cap 256 KiB; needs key header |
| `GET /health` | ingest 127.0.0.1:8787 | Liveness check (`{"ok": true, ...}`) |
| Operator portal | `127.0.0.1:8788` via Tailscale | Browse stored reports |

Environment variables:

| Variable | Default | Meaning |
|----------|---------|---------|
| `LEGION_TELEMETRY_KEY` | *(unset)* | Shared secret; ingest rejects requests without it (401). Set it before exposing the endpoint on the WAN. |
| `LEGION_TELEMETRY_DB` | `diagnostics.db` next to `app.py` | SQLite database path (WAL mode + busy timeout) |
| `LEGION_TELEMETRY_HOST` | `127.0.0.1` | Bind address of the ingest app (loopback; Caddy fronts it) |
| `LEGION_TELEMETRY_PORT` | `8787` | Ingest port |
| `LEGION_TELEMETRY_RATE_PER_MIN` | `30` | Sliding-window POSTs per client IP per minute; beyond that → HTTP 429 |
| `LEGION_TELEMETRY_RETENTION_DAYS` | `90` | Rows older than this are deleted by the hourly prune loop |

## Quickstart

On the VPS:

```bash
./deploy/deploy.sh      # installs the systemd units + Caddy site for $LEGION_WAN_DOMAIN
curl -s http://127.0.0.1:8787/health          # ingest is up
# operator: open http://127.0.0.1:8788 over Tailscale
```

`deploy/deploy.sh` writes the environment shown above into the service units;
edit there (or in its environment file) rather than hand-editing the unit
files.

## Key rotation

```bash
openssl rand -hex 32        # 1. new secret
```

1. Put the new value into `LEGION_TELEMETRY_KEY` wherever
   `deploy/deploy.sh` persists it, then re-run the script — or restart the
   ingest unit after editing its environment.
2. The change takes effect immediately; the old key stops working at restart.
3. Publish the new secret to alpha testers, who set `LEGION_TELEMETRY_KEY`
   on their machines. Until a tester updates their side, sends fail with
   HTTP 401 — the client surfaces the error and nothing is buffered
   server-side; the next successful send carries current data.

## Backup

Use SQLite's online backup API — safe while WAL is active and the ingest
keeps running:

```bash
sqlite3 "$LEGION_TELEMETRY_DB" ".backup '/backups/diagnostics-$(date +%F).db'"
```

A daily cron entry is sufficient; each backup is a small self-contained DB
file. To restore: stop both units, replace the database file (delete any
stale `-wal`/`-shm` siblings first), start the units again.

Manual retention trimming, if you ever need it ahead of the hourly prune:

```sql
DELETE FROM reports WHERE ts < datetime('now', '-90 days');
```

## What is stored

Exactly what the client whitelists — nothing else arrives in the payload:

| Field group | Contents |
|-------------|----------|
| envelope | `schema_version` (1), `generated_at` (UTC RFC 3339), `app_version` |
| device | model name, machine type, BIOS version/prefix, CPU and GPU names, EC firmware version |
| os | distro name + version, kernel release |
| sensors | hwmon/sysfs/NVIDIA readings (temperatures, power, clocks, utilization) |
| battery | capacity %, status, voltage, cycle count, health %, charge-limit % — no serial |
| fans | per-fan id/title, min/max RPM range, current RPM, target RPM |
| thermal | throttle-governor config (enabled, max-temp target), current max CPU frequency |
| curve_optimizer | Curve Optimizer status/values (ryzen_smu path, when present) |
| profiles | current platform-profile name and available choices |
| settings digest | lighting mode, keyboard layout kind, restore-on-launch flag — preference kinds only |
| daemon_log_tail | capped recent daemon log lines; sanitized at write time and home-path-redacted (`/home/*`, `/run/user/*` → `~`) |
| self_checks | pass/fail result per built-in check |

**Not stored:** client IP addresses (rate limiting keeps an in-memory window
only), User-Agent or other HTTP headers, hostname, username, serial numbers
(machine/battery/disk), MAC addresses, per-key colour maps, named profiles,
or any free-form user content.
