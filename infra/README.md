# infra

Deployment infrastructure for the VibeFi IPFS relay service.

## Layout

```
infra/
├── docker-compose.yml      # relay + kubo + caddy
├── Caddyfile               # TLS termination, reverse proxy to relay:8080
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
./setup.sh <SERVER_HOST_OR_IP> root ~/.ssh/your_key
```

The playbook will:
1. Update the OS and install base packages
2. Install Docker CE + Compose plugin from Docker's official repo
3. Configure UFW firewall (ports 22, 80, 443, 4001)
4. Sync your current local checkout to `/opt/ipfs-relay` (default)
5. Write `.env` from inventory variables
6. Build the relay Docker image and start all three services
7. Install + enable the systemd unit for auto-start on reboot

Note: during provisioning, health is verified from inside the relay container
so deploys succeed even before DNS/TLS for `ipfs.vibefi.dev` is live.

If you prefer pulling from GitHub instead of syncing local files, pass:
`-e deploy_source=git -e app_branch=<branch>`

### Point Domain Before Running E2E Tests

After provisioning succeeds:

1. Create/Update DNS records so your domain points to the server.
2. Ensure inbound ports `80` and `443` are reachable at the server/public firewall level.
3. Wait for certificate issuance and HTTPS readiness.

Verify:

```bash
curl -i https://<your-domain>/health
```

Expected result: `HTTP 200` with a JSON health body.

If you use a proxy/CDN (for example Cloudflare), set the record to DNS-only until
origin cert issuance completes, then re-enable proxy mode.

### Run Integration Tests Against The Deployed Domain

From repo root:

```bash
VIBEFI_RELAY_E2E_BASE_URL=https://<your-domain> \
cargo test --test upload_e2e -- --ignored
```

Optional knobs:

```bash
# Enable Kubo CID/content verification checks in remote mode
VIBEFI_RELAY_E2E_KUBO_API_URL=http://<kubo-host>:5001
```

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
