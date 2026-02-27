#!/bin/sh
# Kubo entrypoint: initialise the repo on first run, apply config, then start.
set -e

IPFS_PATH="${IPFS_PATH:-/data/ipfs}"
export IPFS_PATH

# ── First-run initialisation ──────────────────────────────────────────────────
if [ ! -f "${IPFS_PATH}/config" ]; then
    echo "[kubo-init] Initialising IPFS repo (server profile)..."
    ipfs init --profile=server
fi

# ── API: listen on all interfaces so relay container can reach it ─────────────
# The port is NOT exposed to the host (see docker-compose.yml).
ipfs config Addresses.API /ip4/0.0.0.0/tcp/5001

# ── Gateway: internal only ────────────────────────────────────────────────────
ipfs config Addresses.Gateway /ip4/0.0.0.0/tcp/8080

# ── CORS: allow relay service (and local tools) to call the API ───────────────
ipfs config --json API.HTTPHeaders.Access-Control-Allow-Origin \
    '["http://relay:8080", "http://localhost:5001", "http://127.0.0.1:5001"]'
ipfs config --json API.HTTPHeaders.Access-Control-Allow-Methods \
    '["PUT", "POST", "GET"]'
ipfs config --json API.HTTPHeaders.Access-Control-Allow-Headers \
    '["Authorization"]'

# ── Swarm: announce public IP if provided ────────────────────────────────────
if [ -n "${KUBO_PUBLIC_IP}" ]; then
    echo "[kubo-init] Setting swarm announce address to ${KUBO_PUBLIC_IP}"
    ipfs config --json Addresses.Announce \
        "[\"\/ip4\/${KUBO_PUBLIC_IP}\/tcp\/4001\", \"\/ip4\/${KUBO_PUBLIC_IP}\/udp\/4001\/quic-v1\"]"
fi

# ── Peering: well-known bootstrap peers (protocol labs + cloudflare) ──────────
# These are already included in the default bootstrap list; this is a no-op
# unless you wiped it. Kept here for documentation.
# ipfs bootstrap add default

# ── GC: enable automatic garbage collection ───────────────────────────────────
# Keeps disk usage bounded; pinned content is never collected.

echo "[kubo-init] Starting ipfs daemon..."
exec ipfs daemon --migrate=true --enable-gc --routing=dhtclient
