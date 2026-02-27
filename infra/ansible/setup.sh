#!/usr/bin/env bash
# Quick-start wrapper for the Ansible provisioning playbook.
#
# Usage:
#   ./setup.sh <SERVER_IP> [USER] [SSH_KEY_PATH] [extra ansible args...]
#
# Examples:
#   ./setup.sh 1.2.3.4                                    # root + ~/.ssh/id_ed25519
#   ./setup.sh 1.2.3.4 root ~/.ssh/vibefi_deploy          # explicit key
#   ./setup.sh 1.2.3.4 root ~/.ssh/vibefi_deploy --check  # dry-run

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Args ──────────────────────────────────────────────────────────────────────
SERVER_IP="${1:?Usage: $0 <SERVER_IP> [USER] [SSH_KEY_PATH] [extra ansible args...]}"
ANSIBLE_USER="${2:-root}"
SSH_KEY="${3:-}"
shift 3 2>/dev/null || shift $# 2>/dev/null || true
EXTRA_ARGS=("$@")

# ── Resolve SSH key ───────────────────────────────────────────────────────────
if [ -z "${SSH_KEY}" ]; then
    # Try common key names in order
    for candidate in ~/.ssh/id_ed25519 ~/.ssh/id_rsa ~/.ssh/id_ecdsa; do
        if [ -f "${candidate}" ]; then
            SSH_KEY="${candidate}"
            break
        fi
    done
fi

if [ -z "${SSH_KEY}" ]; then
    echo "Error: no SSH key found. Pass one explicitly:"
    echo "  $0 ${SERVER_IP} ${ANSIBLE_USER} ~/.ssh/your_key"
    exit 1
fi

if [ ! -f "${SSH_KEY}" ]; then
    echo "Error: SSH key not found: ${SSH_KEY}"
    exit 1
fi

echo "==> Using SSH key: ${SSH_KEY}"

# ── Dependency checks ─────────────────────────────────────────────────────────
if ! command -v ansible-playbook &>/dev/null; then
    echo "Error: ansible-playbook not found."
    echo "  pip install ansible"
    exit 1
fi

# Install required Ansible collections if missing
ansible-galaxy collection install community.docker --upgrade -q

# ── Build inventory ───────────────────────────────────────────────────────────
TMPINV="$(mktemp /tmp/ipfs-relay-inventory.XXXXXX.ini)"
trap 'rm -f "${TMPINV}"' EXIT

cat > "${TMPINV}" <<INI
[relay]
target ansible_host=${SERVER_IP}

[relay:vars]
ansible_user=${ANSIBLE_USER}
ansible_ssh_private_key_file=${SSH_KEY}
ansible_ssh_common_args=-o StrictHostKeyChecking=accept-new
INI

# ── Run playbook ──────────────────────────────────────────────────────────────
echo "==> Provisioning ${ANSIBLE_USER}@${SERVER_IP} ..."
ansible-playbook \
    -i "${TMPINV}" \
    "${SCRIPT_DIR}/playbook.yml" \
    "${EXTRA_ARGS[@]}"

echo ""
echo "==> Done! Service should be running at https://ipfs.vibefi.dev"
echo "    Health: curl http://${SERVER_IP}:8080/health"
