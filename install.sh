#!/usr/bin/env bash
# Wrapper — run the installer from the monorepo root.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$ROOT/lenovo-legion-tool/install.sh" "$@"
