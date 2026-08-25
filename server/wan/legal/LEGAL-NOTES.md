# Legal Notes — Telemetry Consent Architecture

**Status:** 25 August 2026 · Operator: Adrian Kozlowski (Mannheim)

## Chosen model: prominent first-run choice (explicit opt-in)

On first launch the welcome dialog asks verbatim:

> Share ONE anonymized report occasionally (hardware model, distro, sensors,
> fan/battery stats, self-check results)? Never: hostname · username ·
> serials · MACs · IPs · key colors.

Two equal-weight responses: **Share ✓** / **Not now**. The choice is stored
(`diagnostics.enabled`), changeable anytime under Setup → Alpha diagnostics,
and "Send now" is disabled until the switch is on.

## Why hard opt-out was rejected

An auto-sending variant was evaluated and declined:

1. The payload's only fragile field is the log digest. While raw lines never
   leave the machine and paths are redacted, a *future* log message could
   still embed something identifying; under opt-out that would constitute
   processing of personal data without a legal basis.
2. Under opt-in, the same incident is a contained bug: the data was provided
   voluntarily for exactly that purpose.
3. German Abmahnung exposure for auto-phone-home telemetry in a FOSS alpha
   is disproportionate to the data value.

If opt-out is ever revisited: drop the log digest from the payload entirely,
treat all fields as personal-data-adjacent in docs, run and document an
Art. 6(1)(f) balancing test, and re-verify the anonymity unit test against
real-world logs.

## GDPR positioning

The report contains no personal data (Art. 4 No. 1 GDPR): no identifier can
link a report to a natural person, directly or indirectly (Recital 26 —
anonymised data). GDPR therefore does not apply to collection/storage.
Independent of that necessity argument, collection is voluntary with
explicit one-click consent, withdrawal is immediate via the same switch,
and transport/storage are documented in
[PRIVACY-TELEMETRY.md](PRIVACY-TELEMETRY.md) /
[DATENSCHUTZ-TELEMETRIE.md](DATENSCHUTZ-TELEMETRIE.md).

## Residual operator obligations

1. Secure the collector host + DB (hardened unit, key header, rate limit —
   implemented); rotate `LEGION_TELEMETRY_KEY` if it leaks.
2. Keep retention pruning active (90 days).
3. When shipping alpha publicly, link PRIVACY-TELEMETRY.md from the download
   page and mirror its content into the website Datenschutzerklärung.
4. Never add fields outside the whitelist in `src/diagnostics.rs` without
   updating both privacy documents and the anonymity unit test.
