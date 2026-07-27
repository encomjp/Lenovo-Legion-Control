#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE="$SCRIPT_DIR/package"

command -v kpackagetool6 >/dev/null 2>&1 || {
  echo "kpackagetool6 is required (install KDE Plasma 6 / KPackage)." >&2
  exit 1
}
[[ -f "$PACKAGE/metadata.json" ]] || {
  echo "Widget package is missing: $PACKAGE" >&2
  exit 1
}

if kpackagetool6 --type Plasma/Applet -i "$PACKAGE" 2>/dev/null; then
  echo "Legion Control widget installed."
else
  kpackagetool6 --type Plasma/Applet -u "$PACKAGE"
  echo "Legion Control widget updated."
fi

echo "Add 'Legion Control' from Plasma's widget picker. Plasma was not restarted."
