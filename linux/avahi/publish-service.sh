#!/usr/bin/env bash
# Publish OpenDroidMic mDNS service via Avahi
# Usage: ./publish-service.sh [port]
#
# Option 1 (static, recommended):
#   sudo cp avahi/opendroidmic.service /etc/avahi/services/
#   sudo systemctl restart avahi-daemon
#
# Option 2 (dynamic, this script):
#   ./avahi/publish-service.sh 38471

set -euo pipefail

PORT="${1:-38471}"
SERVICE_TYPE="_opendroidmic._udp"
SERVICE_NAME="OpenDroidMic"

echo "Publishing ${SERVICE_NAME} on port ${PORT}..."

# Kill any previous publish
pkill -f "avahi-publish.*${SERVICE_TYPE}" 2>/dev/null || true

avahi-publish \
    -s "${SERVICE_NAME}" \
    "${SERVICE_TYPE}" \
    "${PORT}" \
    version=1 \
    protocol=odmc
