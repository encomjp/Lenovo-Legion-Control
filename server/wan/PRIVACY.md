# Legion Control alpha diagnostics — privacy statement

Legion Control is free, community-developed Linux control software for Lenovo
Legion laptops. During the alpha phase it can send **one anonymized
diagnostic report** to the project developer, to learn which hardware and
settings real machines use and which built-in self-checks fail in the field.

**Telemetry is off by default.** Nothing is collected or sent until you
switch on “Share anonymous diagnostics” in **Settings → Setup**.

## What is collected

Each report is assembled from a fixed field list enforced in the source
([`src/diagnostics.rs`](../../src/diagnostics.rs)):

- App version and UTC report time.
- Hardware identity: model name, machine type, BIOS version, CPU/GPU names, EC firmware version.
- Operating system: distribution name/version and kernel release.
- Sensor readings (temperatures, power draw, clocks) and fan names/speeds.
- Battery health figures: capacity, status, voltage, cycles, health %, charge limit — no serial number.
- Current CPU thermal-throttle and Curve Optimizer settings (numbers only).
- A small settings digest: lighting mode, keyboard layout kind, restore-on-launch flag. No named profiles, no colour data.
- A capped excerpt of the Legion Control daemon log, scrubbed of home-directory paths.
- Built-in self-check results (one pass/fail per check).

## What is never collected

Hostname, username, account or hardware serial numbers, MAC addresses, IP
addresses, per-key keyboard colour maps, your profiles, files, or typed
content. The report reads only the fields listed above; nothing else is
opened.

## Storage and retention

Reports arrive at the developer’s server, pass a size check, and are appended
to a private SQLite database — one row per report (UTC timestamp + the JSON
payload). Rows older than **90 days** are deleted automatically, and the
operator can remove individual reports at any time. Client IP addresses are
used only for short-lived in-memory rate limiting and are not stored.

## Transport

A report leaves your machine as a single HTTPS request through a TLS reverse
proxy, carrying a shared-secret header so third parties cannot inject fake
reports. To inspect exactly what would be sent — before enabling anything:

```bash
legion-cli diagnose dump   # prints the full anonymous JSON, no upload
```

## Opting out

Switch off “Share anonymous diagnostics” in **Settings → Setup**, or
uninstall Legion Control. Copies already received age out via the 90-day
retention.

## Questions

Open an issue at <https://github.com/encomjp/lenovo-legion-tool/issues>.
