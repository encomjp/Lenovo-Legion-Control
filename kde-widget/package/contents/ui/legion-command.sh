#!/usr/bin/env bash
# Execute legion-cli from the locations supported by the installer or PATH.
# Only a fixed whitelist of subcommands/arguments is forwarded; anything
# else is rejected (exit 64) before legion-cli is ever started.
set -u

allowed=0
case "$#:${1:-}" in
  1:status|1:info|1:profile|1:battery|1:fan|1:kbd|1:fan-auto) allowed=1 ;;
  1:fn-lock|1:gpu-mode) allowed=1 ;;
  2:set-profile) [[ "${2:-}" =~ ^(quiet|balanced|performance|max-power|custom)$ ]] && allowed=1 ;;
  2:charge-limit) [[ "${2:-}" =~ ^(60|80|100)$ ]] && allowed=1 ;;
  2:set-kbd) [[ "${2:-}" =~ ^[0-2]$ ]] && allowed=1 ;;
  2:fn-lock) [[ "${2:-}" =~ ^(on|off)$ ]] && allowed=1 ;;
  3:set-fan)
    [[ "${2:-}" =~ ^[12]$ && "${3:-}" =~ ^(0|3000|3500|4000|4500)$ ]] && allowed=1 ;;
esac
if [[ "$allowed" != 1 ]]; then
  printf 'legion-command.sh: command not allowed\n' >&2
  exit 64
fi

if command -v legion-cli >/dev/null 2>&1; then
  exec "$(command -v legion-cli)" "$@"
fi

for cli in "${HOME:-}/.local/bin/legion-cli" /usr/local/bin/legion-cli /usr/bin/legion-cli; do
  if [[ -x "$cli" ]]; then
    exec "$cli" "$@"
  fi
done

printf 'Legion CLI is not installed.\n' >&2
exit 127
