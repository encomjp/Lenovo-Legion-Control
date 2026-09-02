# Legion Telemetry Collector

Tiny FastAPI service (stdlib otherwise) that receives the **anonymized alpha
diagnostics** reports from opt-in Legion Control builds (`POST
/v1/diagnostics`) and appends them to a local SQLite database
(`diagnostics.db`). One row per report: UTC ISO timestamp + raw JSON string.

## What it collects

Exactly what the client whitelists in its report — privacy contract enforced
in [`lenovo-legion-tool/src/diagnostics.rs`](../lenovo-legion-tool/src/diagnostics.rs):

- hardware model / type / BIOS / CPU / GPU / EC identity
- distro + kernel version
- sensor readings, fan states
- battery health stats (capacity, cycles, health %, charge limit — **no serial**)
- thermal & Curve Optimizer configuration, settings digest
- sanitized daemon log tail, self-check results

**Never collected:** hostname, username, serial numbers, MAC or IP
addresses, per-key colour maps.

## Run manually

```bash
pip install fastapi uvicorn
python3 collector.py     # binds LEGION_TELEMETRY_HOST:LEGION_TELEMETRY_PORT (default 127.0.0.1:8787)
curl http://127.0.0.1:8787/health   # -> {"ok": true, "count": <n>}
```

## Tests

Regression tests for the request path — gzip acceptance (the client sends
`Content-Encoding: gzip`), corrupt/bomb guards, schema gate, verbatim storage:

```bash
pip install pytest fastapi uvicorn httpx
python3 -m pytest server/test_collector.py -v
```

## Run under systemd

Place the file, then enable the unit:

```bash
sudo mkdir -p /opt/legion-telemetry
sudo cp collector.py /opt/legion-telemetry/
```

`/etc/systemd/system/legion-telemetry.service`:

```ini
[Unit]
Description=Legion Telemetry Collector

[Service]
ExecStart=/usr/bin/python3 /opt/legion-telemetry/collector.py
WorkingDirectory=/opt/legion-telemetry
Restart=on-failure
Environment=LEGION_TELEMETRY_HOST=127.0.0.1

[Install]
WantedBy=multi-user.target
```

Dependencies must be visible to `/usr/bin/python3` (`sudo pip install
fastapi uvicorn`) or point `ExecStart` at a venv interpreter instead.

## Public rollout

During alpha the collector binds only to a Tailscale IP. To go public, front
it with nginx + TLS:

```nginx
location /v1/diagnostics {
    proxy_pass http://127.0.0.1:8787;
}
```

Then obtain a certificate (`certbot --nginx -d your.domain`) and move
clients over: they override the endpoint via their config, or you update the
compiled-in `DEFAULT_ENDPOINT` constant in
[`lenovo-legion-tool/src/diagnostics.rs`](../lenovo-legion-tool/src/diagnostics.rs)
(currently `http://127.0.0.1:8787/v1/diagnostics`).

## Retention

You operate the database and act as data controller — prune old reports
regularly, e.g.:

```sql
DELETE FROM reports WHERE ts < datetime('now','-90 days');
```
