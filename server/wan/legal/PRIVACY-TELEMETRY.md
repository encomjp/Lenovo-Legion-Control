# Privacy Statement — Telemetry in the Alpha Version (Legion Control)

**Last updated:** 25 August 2026 · **Applies to:** Legion Control builds 0.1.0-alpha and later · [Deutsche Version](DATENSCHUTZ-TELEMETRIE.md)

## 1. Who is responsible?

The operator of the telemetry collector (Adrian Kozlowski, Mannheim — full
contact details in the main site imprint). Processing happens exclusively on
IONOS server infrastructure (data centre Germany).

## 2. What is collected?

Only after you pick **Share ✓** in the first-launch welcome dialog (or later
enable **Settings → Setup → “Share anonymous diagnostics”**), the software
transmits **one** anonymised JSON report per click on “Send now”:

- **Device:** model name, machine type, BIOS version, CPU/GPU model, EC chip
- **OS:** distribution and kernel version
- **Sensors:** temperatures, fan RPMs and targets, fan limits, battery state
  (capacity, health, cycles — **no** serial number)
- **Configuration:** thermal limit, Curve Optimizer status (values only),
  power scheme
- **App state:** settings digest (lighting mode, keyboard layout) and a
  **log summary** (warn/error counts plus the last error message — home
  paths technically redacted). Raw log lines never leave your machine.
- **Self-tests:** the result list of the built-in diagnostics

## 3. What is NEVER collected?

Hostname · username · IP address (discarded on receipt, never stored) ·
serial numbers · MAC addresses · keyboard layouts/colours · your own profile
names · file contents.

## 4. Legal classification

The transmitted data contains **no personal data** within the meaning of
Art. 4 No. 1 GDPR: it cannot be linked to a natural person, directly or
indirectly (Recital 26 GDPR — anonymised data). The GDPR therefore does
**not apply** to its collection and storage. Independently of that:

- Transmission happens **only after explicit opt-in** (default: off).
  Withdraw anytime via the same switch; previously sent reports remain until
  the retention period ends.
- Transport encryption via HTTPS (Cloudflare edge TLS + origin certificate).
- Storage: IONOS VPS, accessible only to the operator.

## 5. How long is it stored?

Reports are automatically deleted after **90 days** (hourly cleanup job).
Earlier deletion of a specific report can be requested via a GitHub issue.

## 6. How can I verify all of this?

`legion-cli diagnose dump` shows exactly what would be sent — locally,
before sending. Source code is fully open: `src/diagnostics.rs` (the
whitelist) and `server/wan/app.py` (storage).
