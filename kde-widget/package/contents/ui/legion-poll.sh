#!/usr/bin/env bash
# Legion Control sensor poller — outputs key=value lines for the QML widget.
#
# Cost matters: this runs every few seconds inside Plasma. Exactly THREE
# legion-cli invocations per tick (status / profile / battery) — temps,
# powers, and fan RPMs are all parsed from the single `status` dump, and
# kbd/logo were dropped entirely (the widget never displayed them).
# Plus at most ONE conditional `nvidia-smi` query (only when the daemon's
# dGPU reading is missing/negative — see below). Optional values are
# allowed to be absent.
set -u
export LC_ALL=C

# Find legion-cli: check PATH first, then common install paths.
CLI=""
if command -v legion-cli >/dev/null 2>&1; then
  CLI="$(command -v legion-cli)"
else
  for p in "${HOME:-}/.local/bin/legion-cli" /usr/local/bin/legion-cli /usr/bin/legion-cli; do
    [[ -x "$p" ]] && CLI="$p" && break
  done
fi
[[ -z "$CLI" ]] && { echo "LEGION_CLI_NOT_FOUND=1"; exit 0; }

status="$(timeout 3 "$CLI" status 2>/dev/null || true)"
if [[ -z "$status" ]]; then
  echo "LEGION_DAEMON_OFFLINE=1"
  exit 0
fi
printf 'LEGION_OK=1\n'
profile="$(timeout 3 "$CLI" profile 2>/dev/null | grep -v '^[0-9]\{4\}-' | head -1 | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' || true)"
battery="$(timeout 2 "$CLI" battery 2>/dev/null || true)"

value() {
  local key="$1" val="$2"
  [[ -n "$val" ]] && printf '%s=%s\n' "$key" "$val"
}

# Temps / powers / fans — one dump, parsed once.
# Anchors: fields are right-aligned (spaces vary), so require \s+ and a
# trailing °C; accept both the current "CPU" label and the legacy "Tctl".
cpu_t="$(printf '%s\n' "$status" | grep -oP '(Tctl|CPU)\s+\K[0-9.]+(?=°C)' | head -1 || true)"
cpu_p="$(printf '%s\n' "$status" | grep -oP 'CPU power\s+\K[0-9.]+' | head -1 || true)"
dgpu_t="$(printf '%s\n' "$status" | grep -oP 'dGPU\s+\K[-0-9.]+' | head -1 || true)"
dgpu_p="$(printf '%s\n' "$status" | grep 'dGPU' | grep -oP '[-0-9.]+(?=\s+W)' | head -1 || true)"
# Daemon cgroup can block NVML (nvidia-caps) even when the GPU is awake,
# so the daemon may report dGPU -1 while nvidia-smi works fine from this
# user process (same fallback the GUI app does in overview.rs). Only pay
# for the extra subprocess when the daemon value is missing or negative.
if [[ -z "${dgpu_t:-}" || "$dgpu_t" == -* || -z "${dgpu_p:-}" || "$dgpu_p" == -* ]]; then
  smi=""
  if command -v nvidia-smi >/dev/null 2>&1; then
    smi="$(command -v nvidia-smi)"
  else
    for p in /usr/bin/nvidia-smi /usr/local/bin/nvidia-smi /opt/bin/nvidia-smi; do
      [[ -x "$p" ]] && smi="$p" && break
    done
  fi
  if [[ -n "$smi" ]]; then
    smi_raw="$(timeout 3 "$smi" --query-gpu=temperature.gpu,power.draw --format=csv,noheader,nounits 2>/dev/null | head -1 || true)"
    if [[ -n "$smi_raw" ]]; then
      smi_t="$(printf '%s' "$smi_raw" | cut -d, -f1 | tr -d '[:space:]' || true)"
      smi_p="$(printf '%s' "$smi_raw" | cut -d, -f2 | tr -d '[:space:]' || true)"
      if [[ -z "${dgpu_t:-}" || "$dgpu_t" == -* ]] && [[ "$smi_t" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
        dgpu_t="$smi_t"
      fi
      if [[ -z "${dgpu_p:-}" || "$dgpu_p" == -* ]] && [[ "$smi_p" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
        dgpu_p="$smi_p"
      fi
    fi
  fi
fi
value CPU_TEMP "$cpu_t"
value CPU_POWER "$cpu_p"
value DGPU_TEMP "$dgpu_t"
value DGPU_POWER "$dgpu_p"
fans_line="$(printf '%s\n' "$status" | grep -Ei 'rpm' | grep -Ei 'CPU|GPU|Aux' | head -1 || true)"
value FAN_CPU "$(printf '%s\n' "$fans_line" | grep -oP 'CPU\s+\K[0-9]+' | head -1 || true)"
value FAN_GPU "$(printf '%s\n' "$fans_line" | grep -oP 'GPU\s+\K[0-9]+' | head -1 || true)"
value FAN_AUX "$(printf '%s\n' "$fans_line" | grep -oP 'Aux\s+\K[0-9]+' | head -1 || true)"

value PROFILE "$profile"

# Battery basics from sysfs — works even when only the daemon-less parts do.
for bat in /sys/class/power_supply/BAT*; do
  [[ -d "$bat" ]] || continue
  value BATTERY "$(cat "$bat/capacity" 2>/dev/null | tr -d '\n' || true)"
  value BAT_STATUS "$(cat "$bat/status" 2>/dev/null | tr -d '\n' || true)"
  break
done

# Charge limit: the daemon view is authoritative (EC-set limits do not
# always show up in sysfs); fall back to the sysfs heuristic when offline.
limit="$(printf '%s\n' "$battery" | grep -oP 'limit\s+\K[0-9]+' | head -1 || true)"
if [[ -z "$limit" ]]; then
  for bat in /sys/class/power_supply/BAT*; do
    [[ -d "$bat" ]] || continue
    if [[ -f "$bat/conservation_mode" ]] && [[ "$(cat "$bat/conservation_mode" 2>/dev/null)" == "1" ]]; then
      limit=60
    elif [[ -f "$bat/charge_types" ]] && grep -q 'Long_Life' "$bat/charge_types" 2>/dev/null; then
      limit=80
    else
      limit=100
    fi
    break
  done
fi
value CHARGE_LIMIT "$limit"

# Battery power: try power_now (µW), else current_now × voltage_now (µA × µV → W)
bat_power() {
  local bat
  for bat in /sys/class/power_supply/BAT*; do
    [[ -d "$bat" ]] || continue
    if [[ -r "$bat/power_now" ]]; then
      LC_ALL=C awk '{printf "%.1f", $1/1000000}' "$bat/power_now" 2>/dev/null && return
    fi
    if [[ -r "$bat/current_now" && -r "$bat/voltage_now" ]]; then
      local cur vol
      cur="$(cat "$bat/current_now" 2>/dev/null || true)"
      vol="$(cat "$bat/voltage_now" 2>/dev/null || true)"
      if [[ -n "$cur" && -n "$vol" ]]; then
        LC_ALL=C awk -v c="$cur" -v v="$vol" 'BEGIN {printf "%.1f", (c*v)/1000000000000}' 2>/dev/null && return
      fi
    fi
  done
}
value BAT_POWER "$(bat_power || true)"
