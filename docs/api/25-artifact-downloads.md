# Artifact downloads and verification (`/artifacts/:id`, `/artifacts/:id/verify`, `/control/artifacts/:id`)

A job that produces files registers them in an in-process registry
(`harvest::download::ArtifactStore`, capped at 256 artifacts). Downloads are
served **by registry id**, never by path.

```
GET /control/artifacts/:id          → metadata (never bytes, never a path)
GET /artifacts/:id                  → the bytes
GET /artifacts/:id/verify           → re-verify the delivery it belongs to
```

All three are fail-closed on the bearer token, exactly like every other route
except `/health` and `/live`.

## What an artifact record contains

```json
{
  "artifact": {
    "id": "art-3",
    "job": "job-1",
    "name": "records.csv",
    "role": "payload",
    "bytes": 680,
    "sha256": "3c6d4ea4…",
    "content_type": "text/csv; charset=utf-8",
    "state": "ready",
    "downloadable": true,
    "created_ts": 1790066829,
    "verification": { "state": "ok", "files": 4, "records": 4, "records_basis": "jsonl-lines" }
  },
  "job": "job-1",
  "verification": { "state": "ok", "files": 4, "records": 4, "records_basis": "jsonl-lines" }
}
```

Roles: `payload`, `manifest`, `checksums`, `bundle`, and `run_output` (the
configured output file a crawl wrote). There is no `path` field — the server
does not hand out its directory layout, so a client cannot be talked into
asking for something outside a delivery.

## Download guarantees

| Property | How |
|---|---|
| id validation | an id that is not `art-<n>` is `404` before any filesystem call, so `../`, encoded or not, cannot resolve |
| authorization | the bearer token travels in a header; it is never a query parameter, so it cannot leak through a link, a referer or a log |
| size | the file's size is checked against a 256 MiB cap (`MAX_DOWNLOAD_BYTES`) **and** re-checked after reading; an over-cap artifact is `413`, not a truncated body |
| integrity | `x-artifact-sha256` is the hash the registry recorded; `content-length` is the byte count; `content-disposition` carries a bare `filename="…"` |
| verification gate | a delivery whose manifest no longer verifies answers `409` — re-verification is authoritative and re-marks every artifact of that delivery |
| completeness | only finalized, registered files are served; a half-written file is not in the registry |

## Verification

`GET /artifacts/:id/verify` re-runs the same comparison `harvest verify`
performs: presence, byte counts, sha256 per file, and the record total, with
`records_basis` saying where the count came from (`jsonl-lines`,
`manifest-declared`, or `not-checked`).

```json
{ "state": "ok", "delivery": "job-1",
  "detail": { "state": "ok", "files": 4, "records": 4, "records_basis": "jsonl-lines" } }
```

A `failed` verdict is sticky for the artifacts it covers: they stop being
downloadable until a new export replaces the delivery. The CLI remains the
authority — the API reports what `harvest::export_job::verify_dir` decided, it
does not implement a second opinion.

An artifact that is not part of a manifest-backed delivery (the crawl's
`run_output`, for example) is answered `409` on `/verify`: there is nothing to
verify against, and "unknown" is not the same as "ok".
