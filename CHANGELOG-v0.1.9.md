# Changelog - 0.1.9 (2026-08-30)

## Fixed

### Correct discrete-GPU identity on hybrid and APU-only laptops

- GPU discovery now scans PCI display controllers directly, including runtime-suspended devices that may not have a DRM card.
- Installed `nvidia-smi` or NVIDIA kernel modules no longer imply that NVIDIA hardware is present.
- A single AMD or Intel integrated GPU is no longer reported as a discrete GPU.
- `lspci` names with vendor and model bracket pairs now retain the model instead of `AMD/ATI` or a malformed combined string.
- Driver versions are attached only to a matching detected NVIDIA dGPU. Reports also include the dGPU vendor and PCI ID.

### Honest fan RPM self-checks

- Missing RPM support is reported as `backend-unavailable` or `not-exposed`, not as a failed hardware read.
- Exposed but unreadable `fanN_input` attributes still fail the self-check.
- RPM and control backends are resolved separately so a read-only fallback cannot receive target writes.
- Telemetry schema v3 adds separate RPM/control backends and per-fan `readable` and `state` fields.

### Consistent battery telemetry

- The flattened sensor block now uses the same BAT0/BAT1/BAT2/BATT discovery as the canonical battery block, fixing contradictory zero values on systems whose battery is not BAT0.
