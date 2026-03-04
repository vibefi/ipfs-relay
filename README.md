# ipfs-relay

IPFS upload relay service for VibeFi bundles (`ipfs.vibefi.dev`).

Lets users publish VibeFi bundles to IPFS without creating a Pinata/4EVERLAND account. Validates the bundle, pins it to the protocol-owned Kubo node, and queues async replication to protocol-managed pinning providers.

`ipfs.vibefi.dev/ipfs/*` remains the public IPFS gateway path.
For CI deployments to the same Kubo node, infrastructure can optionally expose
`ipfs.vibefi.dev/api/v0/dag/import` with Kubo API authorization enabled.

## Stack

| Layer | Choice |
|---|---|
| Framework | [Axum 0.8](https://github.com/tokio-rs/axum) |
| Runtime | Tokio |
| Tracing | `tracing` + JSON structured logs |
| Metrics | `axum-prometheus` (Prometheus scrape at `/metrics`) |
| Rate limiting | `tower_governor` (per-IP `1/min` and `15/hour`) |
| IPFS | Kubo HTTP API (`/api/v0/add`) |
| Replication | Pinata + 4EVERLAND (async background worker) |

## Endpoints

| Method | Path | Description |
|---|---|---|
| `POST` | `/v1/uploads` | Upload a VibeFi bundle (multipart) |
| `GET` | `/health` | Health check |
| `GET` | `/metrics` | Prometheus metrics |

## Quick start

```bash
# Requires: cargo, a running Kubo node on :5001
cp .env.example .env
cargo run
```

## Configuration

All config can be set via `config/default.toml` or environment variables
prefixed with `VIBEFI_RELAY_`. See `.env.example` for the full list.

Key env vars:

```
VIBEFI_RELAY__IPFS__KUBO_API_URL=http://127.0.0.1:5001
VIBEFI_RELAY__PINNING__PINATA_JWT=<jwt>
VIBEFI_RELAY__PINNING__FOUREVERLAND_TOKEN=<token>
VIBEFI_RELAY__RATE_LIMIT__PER_IP_PER_MINUTE=1
VIBEFI_RELAY__RATE_LIMIT__PER_IP_PER_HOUR=15
```

## Package validation rules

1. Total payload ≤ 10 MiB
2. At least one file present
3. `manifest.json` present and valid JSON
4. `manifest.json` has required fields: `name`, `version`, `createdAt`, `layout`, `entry`, `files`
5. Every `manifest.files[]` entry exists and declared bytes match actual bytes
6. `vibefi.json` present at bundle root
7. `entry` path from manifest exists
8. No absolute paths, `..`, or duplicate logical paths
9. (Optional) `VIBEFI_RELAY__LIMITS__STRICT_MANIFEST=true` rejects files not listed in manifest

## Development

```bash
cargo test
cargo clippy
cargo build --release
```
