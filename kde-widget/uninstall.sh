#!/usr/bin/env bash
set -euo pipefail

ID="com.github.encomjp.legioncontrol"
command -v kpackagetool6 >/dev/null 2>&1 || {
  echo "kpackagetool6 is required (install KDE Plasma 6 / KPackage)." >&2
  exit 1
}

if kpackagetool6 --type Plasma/Applet -r "$ID"; then
  echo "Legion Control widget removed. Plasma was not restarted."
else
  echo "Legion Control widget is not installed." >&2
  exit 1
fi
