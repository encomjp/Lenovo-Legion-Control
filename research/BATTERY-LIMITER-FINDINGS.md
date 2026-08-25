# Battery Charge Limiter — Research Findings & Decision Log

Date: 2026-08-25 · Machine: Legion Pro 7 16AFR10H (83RU, Gen 10)
Status of the code: **fix shipped** in `lenovo-legion-tool` commit `31d71c7`
(single `charge_types` write + read-back verification + `battery-watchdog`).

## Root cause of "charged past the limit"

1. `conservation_mode` and `charge_types` are **two views of one firmware bit**
   (GBMD bit 5 via SBMC args {3=ON, 5=OFF}). Kernel source writes both ops
   inside one `charge_types` store — identical to what Lenovo's Windows Energy
   driver does (IOCTL `0x831020F8`, opcode pairs `{0x5,0x8}`/`{0x8,0x3}`).
   Our old dual-knob sequence was therefore a self-undoing no-op pair.
2. Enforcement is **100 % EC-side**, fire-and-forget from the kernel: no AC-replug
   handler, no retry anywhere upstream. Known failure classes:
   - EC silently clears/garbles state across AC events & suspend
     (kernel Bug 221065 ECMT-mutex issue; Gen-10 owners report charging to 99 %
     even under Windows/Vantage; TLP docs: "some models ignore the setting").
   - Suspend/off trickle-charge bypass documented (charges past threshold while
     asleep; honored only while the OS runs).
3. Firmware threshold is fixed per model (Pro 7 manual: **75–80 %**; older
   IdeaPads 55–60 %) and **cannot be read back or changed** from Linux. "60 %"
   vs "80 %" requests are the same feature on current Legions.

## Decisions made

| Decision | Rationale |
|---|---|
| Write ONLY `charge_types` (Long_Life/Standard) | Standardized, non-deprecated; one write already performs the full SBMC op-pair |
| Verify read-back selection, error on mismatch | Upstream has zero verification; silent success hid real failures |
| `battery-watchdog` re-assert every 5 min | Only practical mitigation for the documented silent-clear; mirrors what Vantage effectively re-does on each Windows boot |
| Collapse sub-100 limits onto the one limiter | Firmware exposes nothing else; labels now say ~75–80 % |

## Rejected / deferred paths

- **`lenovo-wmi-other` WMI battery hook** (GUID `DC2A8805-…`, attr id
  `0x03010001`, hard 80 % cap): present in kernels ≥ 2026-05 but deliberately
  NOT registered when ideapad_laptop owns GBMD/SBMC — which is our case
  (`force_load_psy_ext=1` + blacklisting ideapad_laptop required; driver
  authors warn about state corruption when both run). Treat as test-boot
  experiment only. *Speculation:* on Gen 10 the WMI path may be what the EC
  actually honors.
- **Hysteresis emulation** (LenovoLegionLinux `CustomConservationController`):
  flip limiter OFF below a floor and ON above a ceiling — proven userspace
  fallback where firmware ignores the bit. Candidate v2 if the watchdog proves
  insufficient on 83RU. Could combine with WMI `charge_behaviour=
  force-discharge` (attr `0x03020000`; GET known-buggy upstream) to actively
  drain toward target — would finally give "discharge down" behavior.
- **No `charge_control_end_threshold` exists** on any Legion path; no out-of-tree
  module adds one (firmware takes no percentage input).

## Key references

- torvalds/linux `drivers/platform/x86/lenovo/ideapad-laptop.c` (GBMD/SBMC enums,
  deprecation notice), `wmi-other.c` (battery hook + deconfliction),
  `Documentation/ABI/testing/sysfs-class-power`
- LLL issues #47 (no custom limits; holds instead of discharging), #136 (threshold
  unreadable), #385 (Gen 10 conservation functional)
- LenovoLegionToolkit `Drivers.cs` (EnergyDrv opcodes ≡ SBMC args)
- TLP vendor matrix (bc-vendors.html#lenovo-non-thinkpad-series); TLP #886,
  #882 (BIOS broke charge_types read)
- kernel Bugs 216176 (rapid charge), 221065 (ECMT garbage reads after AC events)
