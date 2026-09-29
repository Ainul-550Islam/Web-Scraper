# AI assistance (optional)

AI assistance is a **stage, not a source of truth**. It runs after deterministic
extraction, proposes values/types/normalizations, and is never authoritative.
It is **off by default**, and nothing in the engine requires it.

```
fetch → deterministic extraction → [ optional AI suggestion ] → transform → schema → dedupe → sink
```

## What it may assist with

* field extraction suggestions for fields the extractor could not fill
* type classification (mapped onto the existing `schema::DeclaredType`
  vocabulary, never a parallel type system)
* normalization suggestions that name **existing** transform ops
  (`parse_number`, `trim`, `collapse_whitespace`, …)
* schema suggestions and semantic labelling

## What it must never do

* bypass CAPTCHA, bot detection, rate limits, authentication, robots, or any
  other security or politeness control. The AI layer sees only values the engine
  already extracted — it has no transport of its own, so it cannot reach a page.
* overwrite a validated deterministic value, unless `overwrite_validated` is
  explicitly enabled.
* destroy data when it fails. A provider timeout, failure or malformed payload
  leaves the record exactly as deterministic extraction produced it.

## Configuration (`harvest::ai::AiCfg`)

| Field | Default | Meaning |
|---|---|---|
| `enabled` | `false` | master switch; false ⇒ every call returns `disabled` |
| `external_allowed` | `false` | consent for providers whose `external()` is true |
| `provider` | `"disabled"` | `deterministic-local`, or an injected provider |
| `timeout_ms` | 2000 | per-call budget |
| `max_retries` | 1 | retries after a transient failure (never for `disabled`) |
| `request_budget` | 100 | requests per client instance (a run/batch) |
| `input_budget_bytes` | 1 MiB | serialized request bytes per instance |
| `thresholds` | 0.90 / 0.60 | accept / review; below review ⇒ reject |
| `overwrite_validated` | `false` | may a suggestion replace a deterministic value |
| `redact_fields` | `[]` | extra field-name fragments to redact |

## Providers

| Provider | External | Notes |
|---|---|---|
| `DisabledProvider` | no | the default; always `AiError::Disabled` |
| `DeterministicProvider` | no | local, network-free, byte-deterministic shape inference (`deterministic-local` / `shape-inference-1`) |
| injected | as declared | implemented by the operator as `AiProvider`; keys stay server-side |

The deterministic provider is honest about what it is: it infers a type from the
*shape* of a value. It is not a hosted model and does not claim to be one.

## Confidence routing

| Band | Confidence | Action |
|---|---|---|
| high | ≥ 0.90 | accept (or hold for review if it would replace a validated value) |
| medium | ≥ 0.60 | mark for review |
| low | < 0.60 | reject / quarantine per the target's schema policy |

`apply_suggestions` reports `accepted`, `review`, `rejected`, `kept_validated`
and `invalid` counts so a run can be audited rather than trusted.

## Privacy and external processing

**Default: no extracted data leaves the process.** An external provider is only
called when `external_allowed` is true. Before any request is built, values whose
field *name* matches a sensitive pattern are replaced with `***`:

```
token · secret · password · passwd · api_key · apikey · authorization · auth
credential · private_key · cookie · session   (+ anything in `redact_fields`)
```

Redacted fields receive **no** suggestion at all — inferring over a marker would
be inferring over nothing.

Provider metadata is limited to name, model, request id, latency and byte counts.
Prompts, API keys, endpoints and raw provider payloads are not part of any type
in this layer, so they cannot be logged, serialized into a delivery, or rendered
in the console.

## State machine

Every outcome carries an explicit state: `disabled`, `unconfigured`,
`ok`, `timeout`, `failed`, `budget-exceeded`, `invalid`. A failed round returns an
empty suggestion list — never a partial application of a bad payload.

## Tests

`tests/ai_pipeline.rs` (21 tests) uses only local providers — no network, no
credentials, so CI runs it offline. It covers: disabled by default, external
consent gating, deterministic classification, request and input-size budgets,
bounded retries (asserted by counting provider calls), timeout, malformed
payload rejection, threshold routing, "AI failure leaves the record intact",
candidate-only filling of missing fields, redaction, and metadata that cannot
carry credentials.
