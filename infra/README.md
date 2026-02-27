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

## Quick deploy (fresh Debian server)

```bash
cd infra/ansible
./setup.sh <SERVER_IP> root ~/.ssh/your_key
```

That's it. The playbook will:
1. Update the OS and install base packages
2. Install Docker CE + Compose plugin from Docker's official repo
3. Configure UFW firewall (ports 22, 80, 443, 4001)
4. Clone this repo to `/opt/ipfs-relay`
5. Write `.env` from inventory variables
6. Build the relay Docker image and start all three services
7. Install + enable the systemd unit for auto-start on reboot

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
