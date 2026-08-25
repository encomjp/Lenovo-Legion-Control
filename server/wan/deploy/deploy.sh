#!/usr/bin/env bash
# Idempotent deploy for the Legion telemetry collector on a Debian/Ubuntu VPS.
#
#   LEGION_WAN_DOMAIN=telemetry.example.com sudo -E ./deploy.sh
#
# Sets up: user, venv, /opt/legion-telemetry tree, systemd units, Caddy site,
# shared key (generated once into /etc/default/legion-telemetry).
set -euo pipefail

LEGION_WAN_DOMAIN="${LEGION_WAN_DOMAIN:?export LEGION_WAN_DOMAIN=<dns-name> first}"
TARGET_USER="${TARGET_USER:-legion-tel}"
APP_DIR="/opt/legion-telemetry"
ENV_FILE="/etc/default/legion-telemetry"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WAN_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"          # .../server/wan
REPO_ROOT="$(cd "${WAN_DIR}/../.." && pwd)"        # repo root

[[ $EUID -eq 0 ]] || { echo "run as root (sudo)"; exit 1; }

echo "→ packages"
apt-get update -qq
apt-get install -y python3-venv caddy >/dev/null

echo "→ user ${TARGET_USER}"
id -u "${TARGET_USER}" >/dev/null 2>&1 || useradd --system --home "${APP_DIR}" --shell /usr/sbin/nologin "${TARGET_USER}"

echo "→ tree ${APP_DIR}"
mkdir -p "${APP_DIR}/server/wan"
rsync -a --delete "${WAN_DIR}/" "${APP_DIR}/server/wan/" \
  --exclude '__pycache__' --exclude 'diagnostics.db*' --exclude 'test_wan.py' --exclude conftest.py
chown -R "${TARGET_USER}:${TARGET_USER}" "${APP_DIR}"

echo "→ venv"
[[ -x ${APP_DIR}/venv/bin/python ]] || python3 -m venv "${APP_DIR}/venv"
"${APP_DIR}/venv/bin/pip" install --quiet --upgrade pip
"${APP_DIR}/venv/bin/pip" install --quiet fastapi uvicorn

echo "→ env file"
if [[ ! -f ${ENV_FILE} ]]; then
  {
    echo "LEGION_TELEMETRY_KEY=$(openssl rand -hex 32)"
    echo "LEGION_TELEMETRY_DB=${APP_DIR}/diagnostics.db"
    echo "LEGION_TELEMETRY_HOST=127.0.0.1"
    echo "LEGION_TELEMETRY_PORT=8787"
    echo "LEGION_WAN_DOMAIN=${LEGION_WAN_DOMAIN}"
  } > "${ENV_FILE}"
  chmod 600 "${ENV_FILE}"
fi
# Portal host is tailnet-only; keep it out of the generic env template above
grep -q LEGION_PORTAL_HOST "${ENV_FILE}" ||
  echo "LEGION_PORTAL_HOST=$(tailscale ip -4 2>/dev/null | head -1 || echo 127.0.0.1)" >> "${ENV_FILE}"

echo "→ systemd units"
cp "${SCRIPT_DIR}/legion-telemetry-ingest.service"  /etc/systemd/system/
cp "${SCRIPT_DIR}/legion-telemetry-portal.service" /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now legion-telemetry-ingest.service
systemctl enable --now legion-telemetry-portal.service

echo "→ caddy site (${LEGION_WAN_DOMAIN})"
install -m 644 "${SCRIPT_DIR}/caddy-legion-telemetry" /etc/caddy/legion-telemetry.caddy
grep -q "import legion-telemetry" /etc/caddy/Caddyfile 2>/dev/null ||
  echo "import /etc/caddy/legion-telemetry.caddy" >> /etc/caddy/Caddyfile
systemctl reload caddy 2>/dev/null || systemctl restart caddy

KEY="$(grep LEGION_TELEMETRY_KEY "${ENV_FILE}" | cut -d= -f2)"
cat <<EOF

════════════════════════════════════════════════════════
Deployed. Tester configuration:

  export LEGION_TELEMETRY_URL="https://${LEGION_WAN_DOMAIN}/v1/diagnostics"
  export LEGION_TELEMETRY_KEY="${KEY}"

Operator portal (Tailscale only):
  http://127.0.0.1:8788/

Verify ingest locally:
  curl -sS -X POST "https://${LEGION_WAN_DOMAIN}/health"
════════════════════════════════════════════════════════
EOF
