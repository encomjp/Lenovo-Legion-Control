#!/usr/bin/env bash
# One-shot static info for the Plasma widget (CPU/GPU names don't change at runtime).
set -u
export LC_ALL=C

CLI=""
if command -v legion-cli >/dev/null 2>&1; then
  CLI="$(command -v legion-cli)"
else
  for p in "${HOME:-}/.local/bin/legion-cli" /usr/local/bin/legion-cli /usr/bin/legion-cli; do
    [[ -x "$p" ]] && CLI="$p" && break
  done
fi
[[ -z "$CLI" ]] && exit 0

info="$(timeout 3 "$CLI" info 2>/dev/null || true)"

value() {
  local key="$1" val="$2"
  [[ -n "$val" ]] && printf '%s=%s\n' "$key" "$val"
}

value CPU_NAME "$(printf '%s\n' "$info" | grep -oP 'cpu\s+\K.*' | head -1 | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' || true)"
value GPU_NAME "$(printf '%s\n' "$info" | grep -oP 'gpu\s+\K.*' | head -1 | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' || true)"
