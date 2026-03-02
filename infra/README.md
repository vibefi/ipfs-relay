# infra

Deployment infrastructure for the VibeFi IPFS relay service.

## Layout

```
infra/
├── docker-compose.yml      # relay + kubo + caddy
├── Caddyfile               # TLS termination, relay proxy + public IPFS gateway paths
├── .env.example            # env vars template (copy to .env)
├── scripts/
│   └── kubo-init.sh        # Kubo first-run init + config entrypoint
├── systemd/
│   └── ipfs-relay.service  # systemd unit (manages docker compose)
└── ansible/
    ├── setup.sh            # quickstart wrapper
    ├── playbook.yml        # full provisioning playbook
    ├── inventory.example.ini
    ├── group_vars/all.yml  # default variables
    └── templates/env.j2   # .env template
```

## Deployment Steps

```bash
cd infra/ansible
./setup.sh <SERVER_IP> [USER] [SSH_KEY_PATH]
```

If `SSH_KEY_PATH` is omitted, `setup.sh` auto-discovers `~/.ssh/id_ed25519`, `~/.ssh/id_rsa`, or `~/.ssh/id_ecdsa` in that order.

Prerequisites on the machine running `setup.sh`:
1. `ansible-playbook` (`pip install ansible`)
2. `gh` CLI
3. `gh auth status -h github.com` is authenticated for this repo

The playbook will:
1. Update the OS and install base packages
2. Create and enable a 1 GB swapfile (`/swapfile`, persisted in `/etc/fstab`)
3. Install Docker CE + Compose plugin from Docker's official repo
4. Configure UFW firewall (ports 22, 80, 443/TCP+UDP, 4001/TCP+UDP)
5. Sync your current local checkout to `/opt/ipfs-relay` (default)
6. Write `.env` from inventory variables
7. Install + enable the systemd unit for auto-start on reboot
8. Build the relay Docker image and start all three services
9. Generate a dedicated GitHub Actions SSH keypair (once) at `infra/ansible/.keys/github_actions_relay_ed25519`
10. Add the generated public key to `authorized_keys` for the deploy user on the server
11. Set GitHub repo secrets via `gh`:
    `RELAY_SERVER_HOST`, `RELAY_SSH_PRIVATE_KEY`, `RELAY_SSH_USER`

Note: during provisioning, health is verified from inside the relay container
so deploys succeed even before DNS/TLS for `ipfs.vibefi.dev` is live.

If you prefer pulling from GitHub instead of syncing local files, pass:
`-e deploy_source=git -e app_branch=<branch>`

### Point Domain Before Running E2E Tests

After provisioning succeeds:

1. Create/Update DNS records so `ipfs.vibefi.dev` points to the server.
2. Ensure inbound ports `80` and `443` are reachable at the server/public firewall level.
3. Wait for certificate issuance and HTTPS readiness.

Verify:

```bash
curl -i https://ipfs.vibefi.dev/health
```

Expected result: `HTTP 200` with a JSON health body.

Public gateway retrieval is exposed at:

```bash
https://ipfs.vibefi.dev/ipfs/<CID>
```

If you use a proxy/CDN (for example Cloudflare), set the record to DNS-only until
origin cert issuance completes, then re-enable proxy mode.

### Run A Single E2E Test Against The Deployed Domain

When running against production, keep rate limiting enabled and run one test that
checks the upload success contract (`201` + response shape) and verifies the CID is
reachable via `/ipfs/<CID>`:
`upload_valid_bundle_returns_201`.

From repo root:

```bash
VIBEFI_RELAY_E2E_BASE_URL=https://ipfs.vibefi.dev \
cargo test --test upload_e2e upload_valid_bundle_returns_201 -- --ignored --exact --nocapture
```

## GitHub Actions

Two workflows are configured:

1. `.github/workflows/e2e-pr.yml`
2. `.github/workflows/release-on-tag.yml`

### PR workflow: local E2E

Trigger: every pull request.

What it does:
1. Starts a local Kubo daemon in Docker on `127.0.0.1:5001`
2. Runs `cargo test --test upload_e2e -- --ignored --nocapture` in local mode

Required GitHub secrets: none.

### Tag workflow: release deploy with Ansible

Trigger: tag push (`*`) and manual `workflow_dispatch`.

What it does:
1. Checks out the tagged commit
2. Runs `infra/ansible/playbook.yml`
3. Deploys using `deploy_source=local` from the checked out workspace

Required GitHub secrets:
1. `RELAY_SERVER_HOST` (set automatically by `setup.sh` / playbook)
2. `RELAY_SSH_PRIVATE_KEY` (set automatically by `setup.sh` / playbook)
3. `RELAY_SSH_USER` (set automatically by `setup.sh` / playbook)

Optional GitHub secrets:
1. `RELAY_PINATA_JWT` (written to `infra/.env` on server)
2. `RELAY_FOUREVERLAND_TOKEN` (written to `infra/.env` on server)
3. `RELAY_KUBO_PUBLIC_IP` (overrides playbook default `ansible_host`)

### Generated deploy keys

`setup.sh`/playbook create a dedicated CI deploy keypair at:
1. `infra/ansible/.keys/github_actions_relay_ed25519`
2. `infra/ansible/.keys/github_actions_relay_ed25519.pub`

This directory is gitignored (`/infra/ansible/.keys/`) and excluded from rsync sync to the server.

## Manual operation

```bash
cd /opt/ipfs-relay/infra

# Status
docker compose ps

# Logs
docker compose logs -f relay
docker compose logs -f kubo

# Rebuild after code change
docker compose up --build -d relay

# Shell into relay
docker compose exec relay sh

# Shell into Kubo
docker compose exec kubo sh
ipfs id
ipfs pin ls
```

## Ports

| Port | Proto | Exposure | Purpose |
|------|-------|----------|---------|
| 80 | TCP | Public | HTTP → HTTPS redirect (Caddy) |
| 443 | TCP | Public | HTTPS (Caddy + Let's Encrypt) |
| 443 | UDP | Public | HTTP/3 QUIC (Caddy) |
| 4001 | TCP+UDP | Public | IPFS swarm (Kubo peer discovery) |
| 5001 | TCP | Internal only | Kubo API (relay → kubo only) |
| 8080 | TCP | Internal only | Relay HTTP (caddy → relay only) |

## Updating

```bash
# On the server
cd /opt/ipfs-relay
git pull
systemctl reload ipfs-relay     # triggers docker compose up --build -d
```

Or re-run the Ansible playbook from your local machine — it's fully idempotent.
