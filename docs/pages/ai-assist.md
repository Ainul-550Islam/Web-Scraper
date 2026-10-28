# AI Assist

| | |
|---|---|
| Route | `/ai` |
| Nav group | Delivery |
| Shortcut | — (none registered) |
| Endpoints | none — file pickers only |
| Polling | none |

Review surface for the **optional** AI stage. Two documents can be loaded: the
operator's provider configuration (shape: `harvest::ai::AiCfg`) and the stage's
output (shape: `harvest::ai::schema::AiOutcome`). Both are read in-tab; this review page does not call an AI endpoint. Bounded
server-side AI jobs are a separate Control Jobs action using `POST /control/ai`;
that route does not turn these local review decisions into engine mutations.

## The three rules the page enforces visually

1. **Off by default.** With no provider document (or with `enabled: false`) the
   page reads `disabled by default` and shows an explicit "AI disabled — no
   suggestions" empty state. Nothing is invented in its place.
2. **External providers need consent.** A provider whose name is a hosted
   service (`openai`, `anthropic`, `azure`, `vertex`, `bedrock`, or a URL) is
   treated as external; with `external_allowed: false` the page shows
   **blocked — no consent** and explains that the engine refuses to send data out
   of the process. The console cannot override that.
3. **Assisted ≠ validated.** Every card carries a provenance badge:

   | Provenance | Badge |
   |---|---|
   | deterministic extraction | `VALIDATED` |
   | provider proposal | `AI SUGGESTION` |
   | proposal a human accepted | `AI CONFIRMED` |

## Confidence bands

Thresholds mirror `harvest::ai::schema::Thresholds` and are shown on the page:

| Band | Confidence | Routing |
|---|---|---|
| high | ≥ 0.90 | may be auto-accepted (never over a validated value unless `overwrite_validated`) |
| medium | ≥ 0.60 | marked for review |
| low | < 0.60 | rejected |

Review actions (Accept / Mark for review / Reject) are **local**: they record a
human decision in this session and can be copied out as a review document. They
do not mutate engine state — there is no endpoint for that, and inventing one
would put an AI decision into the pipeline without an audit trail.

## Privacy

* Field names matching the sensitive patterns (`token`, `authorization`,
  `password`, `secret`, `api_key`, `credential`, `cookie`, `session`, …) are
  replaced with `***` before a request is built — the card refuses to render a
  value for such a field at all.
* Provider metadata carries name, model, request id, latency and byte counts.
  Prompts, keys, endpoints and raw provider payloads are not part of the model,
  so they cannot leak into a card, a log line or a download.
* A failed stage (`failed`, `timeout`, `budget-exceeded`, `invalid`) is reported
  by name with an empty suggestion list — never backfilled with a plausible-looking
  guess.

- **Source:** `web/src/pages/AiAssist.tsx`, `web/src/components/AiSuggestionCard.tsx`
- **Tests:** `web/src/test/ai-assist.test.tsx` (13 tests: disabled default, unconfigured document, blocked external, malformed document, provenance labelling, confidence bands, review flow, failed stage, redaction, overwrite warning, thresholds) · `tests/ai_pipeline.rs` (21 tests on the Rust layer)
- **See also:** [docs/ai-assistance.md](../ai-assistance.md) · [pipeline](pipeline.md) · [data quality](quality.md)
