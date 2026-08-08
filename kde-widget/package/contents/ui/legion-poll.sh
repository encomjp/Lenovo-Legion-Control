#!/usr/bin/env bash
# Legion Control sensor poller — outputs key=value lines for the QML widget.
# Optional sensors are allowed to be absent.
set -u

# Find legion-cli: check common install paths (widget runs in Plasma env, not user shell).
CLI=""
for p in /usr/local/bin/legion-cli /usr/bin/legion-cli "$HOME/.local/bin/legion-cli"; do
  [[ -x "$p" ]] && CLI="$p" && break
done
[[ -z "$CLI" ]] && { echo "LEGION_CLI_NOT_FOUND=1"; exit 0; }

status="$(timeout 3 "$CLI" status 2>/dev/null || true)"
if [[ -z "$status" ]]; then
  echo "LEGION_DAEMON_OFFLINE=1"
  exit 0
fi
printf 'LEGION_OK=1\n'
fans="$(timeout 3 "$CLI" fan 2>/dev/null || true)"

value() {
  local key="$1" val="$2"
  [[ -n "$val" ]] && printf '%s=%s\n' "$key" "$val"
}

value CPU_TEMP "$(printf '%s\n' "$status" | grep -oP 'Tctl\s+\K[0-9.]+' | head -1 || true)"
value IGPU_TEMP "$(printf '%s\n' "$status" | grep -oP 'iGPU\s+\K[0-9.]+' | head -1 || true)"
value DGPU_TEMP "$(printf '%s\n' "$status" | grep -oP 'dGPU\s+\K[0-9.]+' | head -1 || true)"
value DGPU_POWER "$(printf '%s\n' "$status" | grep 'dGPU' | grep -oP '[0-9.]+\s+W' | head -1 | grep -oP '[0-9.]+' || true)"

value FAN_CPU "$(printf '%s\n' "$fans" | grep -oP 'CPU fan:\s+\K[0-9]+' || true)"
value FAN_GPU "$(printf '%s\n' "$fans" | grep -oP 'GPU fan:\s+\K[0-9]+' || true)"
value FAN_AUX "$(printf '%s\n' "$fans" | grep -oP 'Aux fan:\s+\K[0-9]+' || true)"

# Battery is plain sysfs; no daemon round-trip needed.
for bat in /sys/class/power_supply/BAT*; do
  [[ -d "$bat" ]] || continue
  value BATTERY "$(cat "$bat/capacity" 2>/dev/null | tr -d '\n' || true)"
  value BAT_STATUS "$(cat "$bat/status" 2>/dev/null | tr -d '\n' || true)"
  value CHARGE_LIMIT "$(
    if [[ -f "$bat/conservation_mode" ]] && [[ "$(cat "$bat/conservation_mode" 2>/dev/null)" == "1" ]]; then
      echo 60
    elif [[ -f "$bat/charge_types" ]] && grep -q 'Long_Life' "$bat/charge_types" 2>/dev/null; then
      echo 80
    else
      echo 100
    fi
  )"
  break
done

# Battery power: try power_now (µW), else current_now × voltage_now (µA × µV → W)
bat_power() {
  local bat
  for bat in /sys/class/power_supply/BAT*; do
    [[ -d "$bat" ]] || continue
    if [[ -r "$bat/power_now" ]]; then
      awk '{printf "%.1f", $1/1000000}' "$bat/power_now" 2>/dev/null && return
    fi
    if [[ -r "$bat/current_now" && -r "$bat/voltage_now" ]]; then
      awk '{printf "%.1f", ($1*$2)/1000000000000}' "$bat/current_now" "$bat/voltage_now" 2>/dev/null && return
    fi
  done
}
value BAT_POWER "$(bat_power || true)"

value PROFILE "$(timeout 3 "$CLI" profile 2>/dev/null | grep -v '^[0-9]\{4\}-' | head -1 | xargs || true)"
value KBD_BRIGHTNESS "$(timeout 3 "$CLI" kbd 2>/dev/null | grep -oP '\(\K[0-9]+(?=\))' | head -1 || true)"
value LOGO "$(timeout 3 "$CLI" logo 2>/dev/null | grep -oP '(on|off)' | head -1 || true)"
