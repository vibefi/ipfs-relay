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
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

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

# Install required collection only if missing. Avoid forcing a Galaxy call on
# every run, which can fail due to local netrc/credentials policy.
if ! ansible-galaxy collection list community.docker | grep -q "^community.docker "; then
    ansible-galaxy collection install community.docker --upgrade
fi

# ── Build inventory ───────────────────────────────────────────────────────────
# GNU and BSD/macOS `mktemp` use different template rules.
if TMPINV="$(mktemp "${TMPDIR:-/tmp}/ipfs-relay-inventory.XXXXXX" 2>/dev/null)"; then
    :
else
    TMPINV="$(mktemp -t ipfs-relay-inventory)"
fi
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
if [ ${#EXTRA_ARGS[@]} -gt 0 ]; then
    ansible-playbook \
        -i "${TMPINV}" \
        -e "deploy_source=local" \
        -e "local_project_dir=${PROJECT_ROOT}" \
        "${SCRIPT_DIR}/playbook.yml" \
        "${EXTRA_ARGS[@]}"
else
    ansible-playbook \
        -i "${TMPINV}" \
        -e "deploy_source=local" \
        -e "local_project_dir=${PROJECT_ROOT}" \
        "${SCRIPT_DIR}/playbook.yml"
fi

echo ""
echo "==> Done! Service should be running at https://ipfs.vibefi.dev"
echo "    Health (public): curl https://ipfs.vibefi.dev/health"
echo "    Health (on server): ssh ${ANSIBLE_USER}@${SERVER_IP} 'curl -s http://localhost/health'"
