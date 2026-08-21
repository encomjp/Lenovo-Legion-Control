#!/usr/bin/env bash
# Execute legion-cli from the locations supported by the installer.
set -u

for cli in "${HOME:-}/.local/bin/legion-cli" /usr/local/bin/legion-cli /usr/bin/legion-cli; do
  if [[ -x "$cli" ]]; then
    exec "$cli" "$@"
  fi
done

printf 'Legion CLI is not installed.\n' >&2
exit 127
