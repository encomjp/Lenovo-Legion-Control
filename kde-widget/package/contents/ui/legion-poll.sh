#!/usr/bin/env bash
# Legion Control sensor poller — outputs key=value lines for the QML widget.
#
# Cost matters: this runs every few seconds inside Plasma. Exactly THREE
# legion-cli invocations per tick (status / profile / battery) — temps,
# powers, and fan RPMs are all parsed from the single `status` dump, and
# kbd/logo were dropped entirely (the widget never displayed them).
# Optional values are allowed to be absent.
set -u

# Find legion-cli: check common install paths (widget runs in Plasma env, not user shell).
CLI=""
for p in "$HOME/.local/bin/legion-cli" /usr/local/bin/legion-cli /usr/bin/legion-cli; do
  [[ -x "$p" ]] && CLI="$p" && break
done
[[ -z "$CLI" ]] && { echo "LEGION_CLI_NOT_FOUND=1"; exit 0; }

status="$(timeout 3 "$CLI" status 2>/dev/null || true)"
if [[ -z "$status" ]]; then
  echo "LEGION_DAEMON_OFFLINE=1"
  exit 0
fi
printf 'LEGION_OK=1\n'
profile="$(timeout 3 "$CLI" profile 2>/dev/null | grep -v '^[0-9]\{4\}-' | head -1 | xargs || true)"
battery="$(timeout 2 "$CLI" battery 2>/dev/null || true)"

value() {
  local key="$1" val="$2"
  [[ -n "$val" ]] && printf '%s=%s\n' "$key" "$val"
}

# Temps / powers / fans — one dump, parsed once.
# Anchors: fields are right-aligned (spaces vary), so require \s+ and a
# trailing °C; accept both the current "CPU" label and the legacy "Tctl".
cpu_t="$(printf '%s\n' "$status" | grep -oP '(Tctl|CPU)\s+\K[0-9.]+(?=°C)' | head -1 || true)"
value CPU_TEMP "$cpu_t"
value CPU_POWER "$(printf '%s\n' "$status" | grep -oP 'CPU power\s+\K[0-9.]+' | head -1 || true)"
value DGPU_TEMP "$(printf '%s\n' "$status" | grep -oP 'dGPU\s+\K[-0-9.]+' | head -1 || true)"
value DGPU_POWER "$(printf '%s\n' "$status" | grep 'dGPU' | grep -oP '[0-9.]+\s+W' | head -1 | grep -oP '[0-9.]+' || true)"
fans_line="$(printf '%s\n' "$status" | grep -Ei 'CPU.*[0-9]+.*GPU.*[0-9]+' | head -1 || true)"
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
      awk '{printf "%.1f", $1/1000000}' "$bat/power_now" 2>/dev/null && return
    fi
    if [[ -r "$bat/current_now" && -r "$bat/voltage_now" ]]; then
      local cur vol
      cur="$(cat "$bat/current_now" 2>/dev/null || true)"
      vol="$(cat "$bat/voltage_now" 2>/dev/null || true)"
      if [[ -n "$cur" && -n "$vol" ]]; then
        awk -v c="$cur" -v v="$vol" 'BEGIN {printf "%.1f", (c*v)/1000000000000}' 2>/dev/null && return
      fi
    fi
  done
}
value BAT_POWER "$(bat_power || true)"
