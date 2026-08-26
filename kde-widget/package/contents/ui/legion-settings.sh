#!/usr/bin/env bash
# Launch legion-settings from locations supported by the installer or PATH.
set -u

if command -v legion-settings >/dev/null 2>&1; then
  exec "$(command -v legion-settings)" "$@"
fi

for settings in "${HOME:-}/.local/bin/legion-settings" /usr/local/bin/legion-settings /usr/bin/legion-settings; do
  if [[ -x "$settings" ]]; then
    exec "$settings" "$@"
  fi
done

printf 'Legion Settings is not installed.\n' >&2
exit 127
