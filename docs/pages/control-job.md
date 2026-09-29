# Control Job

| | |
|---|---|
| Route | `/control/job?job=<job-id>` |
| Nav group | Delivery |
| Shortcut | — (none registered) |
| Endpoints | `GET /control/jobs/:id`, `POST /control/jobs/:id/cancel`, `GET /artifacts/:id`, `GET /artifacts/:id/verify` |
| Polling | 2 s while the job can still move; 10 minutes once it is terminal |

One control-plane job, as the server describes it. Nothing on the page is
reconstructed client-side: where the API withholds something — a filesystem
path, a command payload, a provider credential — the page is empty there by
design.

## Sections

- **Counters** — pages fetched, records extracted/written, failures, retries,
  quarantined. Zero is zero; the page never estimates.
- **Limits the server applied** — the bounds the job actually ran under, after
  clamping to the attached configuration. Asking for more does not widen them.
- **Artifacts** — one row per registered file with role, bytes, content type and
  sha256, plus **Verify** and **Download**. Both are server calls: the console
  does not hash a file it cannot read, and does not offer a download the server
  would refuse.
- **Result** — the job's own result block (formats rendered, records, the AI
  provider's verdict and counts).
- **Timeline** — `queued → started → completed|failed|cancelled` with
  timestamps, straight from the job's event list.
- **Audit reference** — the id of the server's ledger line for this job. The
  console keeps no audit copy of its own.

## Terminal means terminal

Cancel is disabled once a job is `completed`, `failed` or `cancelled`, because
the server would answer `409`. If the button is somehow still pressed, the 409
is reported as "already finished — nothing to cancel" rather than as a console
error.

## AI jobs have nothing to download

An AI round produces metadata only. The artifacts section says so explicitly
instead of showing an empty table — an empty table would read like a bug.
