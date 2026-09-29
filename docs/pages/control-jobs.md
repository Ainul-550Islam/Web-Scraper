# Control Jobs

| | |
|---|---|
| Route | `/control` |
| Nav group | Delivery |
| Shortcut | — (none registered) |
| Endpoints | `GET /config`, `GET /control/jobs`, `POST /control/crawl`, `POST /control/export`, `POST /control/ai`, `POST /control/jobs/:id/cancel` |
| Polling | 2 s while the page is open (the server pumps jobs on its own schedule) |

Submit **one bounded command**, then watch what the server says about it. The
page is a client for the control plane — it does not run, queue, retry or
schedule anything itself.

## The loop, end to end

| Step | Who does it |
|---|---|
| validate the command, apply ceilings, clamp to the attached config | **server** (`harvest::control`) |
| authenticate, authorize, audit | **server** (`src/admin.rs`, `src/ops.rs`) |
| run the crawl / render the formats / call the provider | **existing engine, exporter and AI client** |
| enforce concurrency (8) and queue depth (64) | **server** (`harvest::execution`) |
| enforce the deadline and cancellation | **server** |
| register artifacts, verify a delivery | **server** (`harvest::download`, `harvest::export_job`) |
| paint states, counters, limits and reasons | **console** (this page) |

## What the page shows

- **Capacity** as the server reports it: `caps.max_concurrent_jobs`,
  `caps.max_queue_depth`, and how many jobs are in flight.
- **States** exactly as the server names them — `queued`, `running`,
  `completed`, `failed`, `cancelled` — with a legend. A running job is
  "running": there is no progress bar, because the engine does not report one.
- **Reasons**: a failed job shows the server's reason verbatim (for example
  `ai assistance is disabled for this job`).
- **Ceilings** in the composer are the server's constants
  (1,000,000 pages, 20 levels, 3,600 s), labelled as such.

## Submissions are safe to retry

Every submission carries an `Idempotency-Key` header generated from the
command. A second identical submission returns the **original** job with
`replayed: true` instead of starting a second crawl; the same key with a
different body is `409`. The key is derived locally and never contains a
secret.

## Honest failure modes

| What happened | What you see |
|---|---|
| no/incorrect token | "The admin API requires a bearer token" (401) |
| queue full | "The control plane is at capacity" (503) — nothing was queued silently |
| command out of bounds | the offending field, before submission (400) |
| cancel on a finished job | "already finished — nothing to cancel" (409) |
