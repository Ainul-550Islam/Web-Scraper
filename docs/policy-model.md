# Policy model (pre-request gates)

Audience: operators tuning `default_policy`/`domains`, reviewers of crawl
ethics, and anyone embedding the decision core. Owner: `src/policy_engine.rs`
(pure decision function) — side effects stay in the engine, budget, breaker,
and robots modules it consumes. Contract-pinned by `tests/policy_regression.rs`.

## The gate order

`evaluate(&PolicyFacts) -> Decision` walks these gates, first failure wins:

1. **target policy** — the URL's domain must be crawlable per config
   (`include`/`exclude`, target scope). Rejection: `deny(target-policy-rejected)`.
2. **robots** — `RobotsVerdict::{Allowed, Denied, NotInspected, Unavailable}`.
   With `respect_robots: true` (the default, and un-reachable-to-false for
   normal configs), `Denied` is **terminal** — no other gate can outvote it.
   `NotInspected` returns `delay(0)` (look first, then ask again);
   `Unavailable` (robots fetch itself failed) allows — the engine's own
   failure policy for unreadable robots.txt is to proceed cautiously, which
   the existing engine suites already pin.
3. **request budget** — `LimitExceeded { budget }` when the run budget says stop.
4. **retry cap** — `RetryExhausted` past `max_retries` (`max_attempts` is
   clamped `.max(1)`; a zero-retry config still means "one attempt", not "zero").
5. **breaker** — `BreakerVerdict::{Closed, Open, Disabled, HalfOpen}`; an open
   breaker yields `deny(reliability-blocked)`, half-open admits probes.
6. **politeness** — token bucket + `min_interval_ms`; waiting is never a
   denial: `delay(wait_ms)`.

Outcomes: `Allow` | `Deny { reason }` | `Delay { wait_ms }` |
`RetryLater { reason }` — and `decision_json(&Decision)` serializes them in a
bounded, two-key shape suitable for the audit journal (no unbounded strings;
reason vocabulary is fixed).

## Safe defaults (the regression table)

`tests/policy_regression.rs` pins defaults **and** refusals:

* `respect_robots` defaults to `true`;
* `max_response_bytes` default 16 MiB — and the suite fails if any tested
  profile lets it exceed 64 MiB;
* retry caps stay ≤ 10; default `requests_per_sec` ≤ 1000 — aggressive
  values belong to explicit per-domain policy, never to defaults;
* config keys that would mean "ignore the rules" (`ignore_robots`,
  `bypass`, `evasion`, `spoof`, `stealth`) are **not** accepted names: the
  top-level key audit in `src/config.rs` rejects unknown keys, so they fail
  closed, loudly, at load time. There is no flag that turns ethics checks off.

## What this module is *not*

It does not sleep, count tokens, or fetch robots.txt — those are the
existing `ratelimit`, `budget`, `breaker`, and `robots` components it takes
as inputs. Engine hot paths are unchanged; the pure function exists so the
decision table above is testable without a network, a clock, or a queue.
