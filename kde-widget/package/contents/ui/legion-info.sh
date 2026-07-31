#!/usr/bin/env bash
# One-shot static info for the Plasma widget (CPU/GPU names don't change at runtime).
set -u

CLI=""
for p in /usr/local/bin/legion-cli /usr/bin/legion-cli "$HOME/.local/bin/legion-cli"; do
  [[ -x "$p" ]] && CLI="$p" && break
done
[[ -z "$CLI" ]] && exit 0

info="$("$CLI" info 2>/dev/null || true)"

value() {
  local key="$1" val="$2"
  [[ -n "$val" ]] && printf '%s=%s\n' "$key" "$val"
}

value CPU_NAME "$(printf '%s\n' "$info" | grep -oP 'cpu\s+\K.*' | head -1 | xargs || true)"
value GPU_NAME "$(printf '%s\n' "$info" | grep -oP 'gpu\s+\K.*' | head -1 | xargs || true)"
