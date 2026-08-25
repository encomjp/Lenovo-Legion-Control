#!/usr/bin/env bash
# Remote bootstrap — AlmaLinux 9 + Cloudflare edition.
#
# TLS model: the telemetry DNS record is PROXIED through Cloudflare, so
# visitors get Cloudflare's free edge certificate. The origin serves a
# Cloudflare Origin Certificate (15-year) or, if the API token lacks the
# ssl_certs permission, a 10-year self-signed fallback — Cloudflare does
# not validate origin certs in "Full" mode. No certbot anywhere.
#
# Executed via: ssh root@HOST 'bash -s' < remote-install.sh
# Requires env: LEGION_WAN_DOMAIN. Optional: LEGION_TELEMETRY_KEY.
set -euo pipefail

LEGION_WAN_DOMAIN="${LEGION_WAN_DOMAIN:?export LEGION_WAN_DOMAIN first}"
LEGION_TELEMETRY_KEY="${LEGION_TELEMETRY_KEY:-$(openssl rand -hex 32)}"
TARGET_USER="legion-tel"
APP_DIR="/opt/legion-telemetry"
ENV_FILE="/etc/default/legion-telemetry"
NGINX_SSL_DIR="/etc/nginx/ssl"
INGEST_PORT=8791          # 8787 taken by docker 'headroom' on this box
PORTAL_HOST="$(tailscale ip -4 2>/dev/null | head -1 || echo 127.0.0.1)"
PORTAL_PORT=8788

[[ $EUID -eq 0 ]] || { echo "must run as root"; exit 1; }

echo "→ packages (AlmaLinux: nginx, venv, openssl; NO certbot)"
dnf install -y nginx openssl >/dev/null

echo "→ firewall: open http/https (edge TLS lives at Cloudflare)"
if command -v firewall-cmd >/dev/null; then
  firewall-cmd --permanent --add-service=http --add-service=https >/dev/null
  firewall-cmd --reload >/dev/null
fi

echo "→ user ${TARGET_USER} + tree ${APP_DIR}"
id -u "${TARGET_USER}" >/dev/null 2>&1 || useradd --system --home "${APP_DIR}" --shell /sbin/nologin "${TARGET_USER}"
mkdir -p "${APP_DIR}/server/wan" "${NGINX_SSL_DIR}"

echo "→ venv"
python3 -m venv "${APP_DIR}/venv"
[[ -x "${APP_DIR}/venv/bin/pip" ]] || { echo "venv creation failed"; exit 1; }
"${APP_DIR}/venv/bin/pip" install --quiet --upgrade pip
"${APP_DIR}/venv/bin/pip" install --quiet fastapi "uvicorn[standard]"

echo "→ SELinux: allow nginx outbound connect (reverse proxy to 127.0.0.1:${INGEST_PORT})"
setsebool -P httpd_can_network_connect 1 2>/dev/null || true

echo "→ env file ${ENV_FILE}"
if [[ ! -f ${ENV_FILE} ]]; then
  {
    echo "LEGION_TELEMETRY_KEY=${LEGION_TELEMETRY_KEY}"
    echo "LEGION_TELEMETRY_DB=${APP_DIR}/diagnostics.db"
    echo "LEGION_WAN_DOMAIN=${LEGION_WAN_DOMAIN}"
    echo "LEGION_PORTAL_HOST=${PORTAL_HOST}"
  } > "${ENV_FILE}"
  ( umask 077 && cat /dev/null > "${ENV_FILE}.permcheck" ) 2>/dev/null || true
  chmod 600 "${ENV_FILE}"
else
  echo "   existing env file preserved"
fi

echo "→ ownership ${TARGET_USER}"
chown -R "${TARGET_USER}:${TARGET_USER}" "${APP_DIR}"

cat <<EOF

════════════════════════════════════════════════════════
BASE BOOTSTRAP DONE. Remaining steps (parent session):
  1. scp app files → ${APP_DIR}/server/wan/
  2. Cloudflare: set DNS record proxied=true + issue Origin Cert
  3. nginx 443 site + systemd units (see deploy/)
Key material lives in ${ENV_FILE}
════════════════════════════════════════════════════════
EOF
