# Browser Studio

| | |
|---|---|
| Route | `/browser-studio` |
| Nav group | Delivery |
| Shortcut | — (none registered) |
| Endpoints | `GET /browser/diagnostics` · `POST /browser/preview` · `POST /browser/selector/test` · `POST /browser/inspect` · `POST /browser/workflow` |
| Polling | none on the page itself; the diagnostics card is re-read after each run |

An authoring surface for browser targets: preview a page you are authorized to
read, point at elements to build a selector tree, test that tree against the live
page, and save the workflow for reuse. It is a **client** — the page owns no
browser state, no crawl queue and no retry policy. Everything it shows is the
answer to a bounded command the server already accepted.

## What the page shows

- **Browser runtime** — whether the feature is compiled in, which engine it is,
  how many browser sessions are active against the configured maximum, and
  whether the runtime is actually attached. When it is not attached the card
  reports the last error rather than a green dot.
- **Current page** — the resolved `final_url`, the HTTP status, the visible-text
  snapshot (first 12,000 characters) and any artifacts the run registered.
- **Element picker** — pick an element out of the fetched HTML and drop its
  selector straight into the tree.
- **Selector tree** — text fields, repeated records and nested children, each
  with a selector, a required flag, a repeat flag and transforms.
- **Records** — what the tree actually extracted, from the selector test.

## The target must be authorized

The composer asks for a complete `http(s)` URL and rejects anything else
before it is sent. The server re-checks the host against the session's
`allowed_hosts`, so a redirect to a host outside that list stops the run — the
studio does not follow the redirect for you.

**Challenges are not bypassed.** If the response reports a challenge, the page
says so and stops for review:

> Challenge detected: `<kind>`. Policy stops for review.

There is deliberately no solver, no fingerprint spoofing and no proxy rotation
here. A challenged target is a signal to a human, not a puzzle to be solved
automatically.

## The workflow loop

| Step | Who does it |
|---|---|
| load / preview a target | server renders it under the session's scope |
| pick elements, build the tree | console |
| test the tree | server extracts against the live page, `max_records` 100 |
| inspect again | server captures the DOM |
| save the workflow | server stores it under a monotonically increasing version |

The version matters: the save carries the version the page last saw, so a save
that races an earlier one is rejected rather than silently overwritten.

## Sessions are per process

Session state lives in the server's memory unless a secure session provider is
configured. A blank session id starts a new isolated session; leaving the id
in place reuses it. Restarting the engine drops it, and the page says so on the
next command instead of failing quietly.

## Honest failure modes

| What happened | What you see |
|---|---|
| feature not compiled in | runtime card reports `feature compiled: no` and the runtime is not attached |
| runtime attached, WebDriver missing | the error text, verbatim, in the runtime row |
| target is not `http(s)` | "Only http(s) URLs are supported." — nothing is sent |
| redirect outside `allowed_hosts` | the server stops the run; the error names the host |
| selector test returns nothing | "Selector test returned 0 records." — the tree, not the page, is wrong |
| save rejected for a stale version | the save's own error; the stored version is unchanged |

- **See also:** [control-jobs](control-jobs.md), [modules/scrape-api](../modules/scrape_api.md), [capability matrix](../capability-matrix.md)
