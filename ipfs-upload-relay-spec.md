# IPFS Upload Relay Spec (`ipfs.vibefi.dev`)

Status: Draft  
Last Updated: 2026-02-27  
Owner: VibeFi protocol/backend

## 1. Purpose

Define a protocol-hosted upload service so users can publish VibeFi bundles to IPFS without creating third-party pinning accounts.

This service is upload-only and is used by the Code `Propose Upgrade` publish flow.

## 2. Scope

In scope:

- Accept bundle uploads from VibeFi clients.
- Return a deterministic root CID.
- Validate that uploads are real VibeFi packages (not arbitrary junk).
- Pin/replicate uploaded content using protocol-managed infra/providers.
- Enforce quotas and abuse controls.

Out of scope:

- IPFS download/gateway behavior (already handled elsewhere).
- On-chain proposal creation itself.
- General-purpose arbitrary file hosting.

## 3. Product Requirements

1. Default upload path in client should be protocol relay (`protocolRelay`).
2. User should not need a Pinata/4EVERLAND account for baseline usage.
3. API should be protocol-native only (no Kubo compatibility requirement).
4. CID returned by relay must be compatible with current bundle verifier/runtime.
5. Service must reject non-package uploads.
6. Service must be abuse-resistant (size caps, rate limiting, auth policy).

## 4. Upload API (Protocol-Native Only)

### `POST /v1/uploads`

Headers:

- `Content-Type: multipart/form-data`
- `Authorization: Bearer <api-key>` optional for now.

Multipart fields:

- Repeated `file` parts.
- Each part filename is the relative path in the bundle (`src/App.tsx`, `manifest.json`, etc).
- No directory traversal (`..`) or absolute paths.

Success response (`201`):

```json
{
  "uploadId": "upl_01J...",
  "rootCid": "bafy...",
  "bytes": 182341,
  "fileCount": 23,
  "validation": {
    "isVibeFiPackage": true
  },
  "pinning": {
    "local": "pinned",
    "replicas": [
      { "target": "pinata", "status": "queued" },
      { "target": "4everland", "status": "queued" }
    ]
  }
}
```

Error response (`4xx/5xx`):

```json
{
  "error": {
    "code": "INVALID_PACKAGE",
    "message": "manifest.json missing required field: files"
  },
  "requestId": "req_..."
}
```

### `GET /v1/uploads/{uploadId}`

Returns processing state and replication status.

```json
{
  "uploadId": "upl_01J...",
  "rootCid": "bafy...",
  "status": "completed",
  "replication": [
    { "target": "local", "status": "pinned" },
    { "target": "pinata", "status": "pinned" },
    { "target": "4everland", "status": "pinned" }
  ]
}
```

## 5. Package Sanity Checks

Validation rules:

1. Total payload size must be `<= 10 MiB`.
2. At least one file must be present.
3. `manifest.json` must exist at bundle root and parse as JSON.
4. `manifest.json` must include required VibeFi fields (`name`, `version`, `createdAt`, `layout`, `entry`, `files`).
5. Every entry in `manifest.files[]` must exist in uploaded files and declared `bytes` must match actual bytes.
6. `vibefi.json` must exist at bundle root.
7. `entry` path from manifest must exist (normally `index.html`).
8. No absolute paths, `..`, or duplicate logical paths.
9. Optional strict mode: reject files not listed in manifest.

If any check fails, return `400 INVALID_PACKAGE`.

## 6. Auth Model (Current)

Current requirement:

- Signature is not required today.

Supported modes now:

1. Anonymous uploads with strict quotas.
2. Optional API key uploads (higher quotas for trusted callers).

Future mode:

- Wallet-signature auth can be added later without changing endpoint shape.

Policy:

- Keep unsigned mode enabled until protocol decides to enforce signed uploads.

## 7. Abuse Controls

Hard limits (initial defaults):

- Max total upload bytes: `10 MiB`
- Max file count: `1500`
- Max single file size: `5 MiB`
- Allowed path rules: UTF-8, no absolute paths, no `..`, no empty segments.

Rate limits (initial defaults):

- Per IP: `30` uploads/hour
- Per API key: `300` uploads/day
- Burst handling via token bucket.

Operational protections:

- Request timeout: `120s`
- Temporary staging quota per instance.
- Optional denylist for abusive addresses/IP ranges.

## 8. Pinning & Replication Pipeline

1. Receive multipart bundle.
2. Run sanity/package validation.
3. Import content into protocol local IPFS node and pin locally.
4. Create replication jobs to protocol-managed providers:
   - Pinata account (protocol-owned)
   - 4EVERLAND account (protocol-owned)
   - Optional additional backup target(s)
5. Persist upload metadata and replication status.
6. Return success after local pin is complete and replication jobs are queued.

Important:

- User credentials for Pinata/4EVERLAND are not required for protocol relay mode.
- Replication failures must not invalidate already returned CID; they should trigger retries/alerts.

## 9. Storage & Metadata

Persist minimal metadata per upload:

- `uploadId`
- `rootCid`
- `sourceIp` (hashed/truncated per policy)
- `authMode` (`anonymous` or `apiKey`)
- `bytes`, `fileCount`
- `createdAt`
- `replicationStatus`
- `requestId`

Retention:

- Keep metadata for abuse/audit and support operations (initial target: `90 days`, configurable).

## 10. Observability

Structured logs:

- `requestId`, `uploadId`, `authMode`, `statusCode`, `durationMs`, `bytes`, `rootCid`.
- Never log bearer tokens, signatures, or full raw multipart bodies.

Metrics:

- `relay_upload_requests_total{auth_mode,status}`
- `relay_upload_bytes_total`
- `relay_upload_duration_seconds`
- `relay_replication_jobs_total{target,status}`
- `relay_replication_queue_depth`

Alerts:

- Elevated `5xx` rate
- Replication backlog growth
- Pin failure ratio above threshold

## 11. Deployment

DNS/TLS:

- Host: `ipfs.vibefi.dev`
- HTTPS required; HTTP redirects to HTTPS.

Runtime components:

- Upload API service
- Local Kubo node or IPFS Cluster gateway
- Queue + worker for async replication
- Metadata DB

Configuration (env):

- `VIBEFI_RELAY_API_KEYS=...`
- `VIBEFI_RELAY_MAX_UPLOAD_BYTES`
- `VIBEFI_RELAY_MAX_FILE_COUNT`
- `VIBEFI_RELAY_PINATA_JWT`
- `VIBEFI_RELAY_4EVERLAND_TOKEN`
- `VIBEFI_RELAY_KUBO_API_URL`

## 12. Client Integration Notes

Client expectation:

- Set Code upload provider to `protocolRelay`.
- Set endpoint to `https://ipfs.vibefi.dev`.
- Client should call `POST /v1/uploads` (not Kubo add).
- Keep fallback providers (4EVERLAND, Pinata, local node) unchanged.

## 13. Acceptance Criteria

1. User can click `Propose Upgrade` with `protocolRelay` selected and receive a valid CID without any third-party account setup.
2. Service rejects uploads over `10 MiB`.
3. Service rejects non-VibeFi bundles (`manifest.json`/`vibefi.json` missing or invalid).
4. Returned CID resolves and passes existing manifest/file verification in client runtime.
5. Service forwards accepted uploads to local IPFS and queues replication to Pinata + 4EVERLAND.
6. Replication jobs are observable and retryable.
7. Secrets are never emitted in logs.

## 14. Open Decisions

1. Final anonymous/API-key quota values.
2. Whether to enable strict rejection of files not listed in `manifest.files`.
3. Timeline for introducing wallet-signature auth.
4. Whether CLI publish should also default to protocol relay once stable.
