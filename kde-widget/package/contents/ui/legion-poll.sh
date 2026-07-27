#!/usr/bin/env bash
# Legion Control sensor poller — outputs key=value lines for the QML widget.
# Optional sensors are allowed to be absent.
set -u

status="$(legion-cli status 2>/dev/null || true)"
fans="$(legion-cli fan 2>/dev/null || true)"
battery="$(legion-cli battery 2>/dev/null || true)"
info="$(legion-cli info 2>/dev/null || true)"

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

value BATTERY "$(printf '%s\n' "$battery" | grep -oP 'battery\s+\K[0-9]+' || true)"
value BAT_STATUS "$(printf '%s\n' "$battery" | grep -oP 'battery\s+[0-9]+%\s+\(\K[^)]+' || true)"
value CHARGE_LIMIT "$(printf '%s\n' "$battery" | grep -oP 'limit\s+\K[0-9]+' || true)"

value PROFILE "$(legion-cli profile 2>/dev/null | grep -v '^[0-9]\{4\}-' | head -1 | xargs || true)"
value KBD_BRIGHTNESS "$(legion-cli kbd 2>/dev/null | grep -oP '\(\K[0-9]+(?=\))' | head -1 || true)"
value LOGO "$(legion-cli logo 2>/dev/null | grep -oP '(on|off)' | head -1 || true)"
value CPU_NAME "$(printf '%s\n' "$info" | grep -oP 'cpu\s+\K.*' | head -1 | xargs || true)"
value GPU_NAME "$(printf '%s\n' "$info" | grep -oP 'gpu\s+\K.*' | head -1 | xargs || true)"
