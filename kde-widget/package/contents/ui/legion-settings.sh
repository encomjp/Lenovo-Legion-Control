#!/usr/bin/env bash
# Launch legion-settings from locations supported by the installer.
set -u

for settings in /usr/local/bin/legion-settings /usr/bin/legion-settings "${HOME:-}/.local/bin/legion-settings"; do
  if [[ -x "$settings" ]]; then
    exec "$settings" "$@"
  fi
done

printf 'Legion Settings is not installed.\n' >&2
exit 127
