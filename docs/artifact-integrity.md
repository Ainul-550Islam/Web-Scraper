# Artifact integrity (export → verify → classify)

Audience: operators shipping exports to third parties, and auditors
re-checking a delivery folder. Owners: `src/export.rs` (writer),
`src/sign.rs` (Ed25519 detach), `src/artifact.rs` (reader/classifier).
Behavior is pinned by `tests/artifact_integrity.rs` (9 tests) and the
pipeline smoke in `tests/final_release.rs`.

## The delivery folder

`harvest export --out DIR inputs…` writes payload parts (optionally split
per target, chunked at a byte cap on record boundaries) and one
`manifest.json`. The manifest never self-lists; `files[]` promises exactly
the payload parts: `{name, sha256, bytes, lines}`. Identity block:
`{v, generated_ts, runs, records, unique_urls, skipped_lines,
policy_sha256}`. Everything in it is derived from the actual bytes written —
the export is deterministic except for `generated_ts` (documented, deliberate).

`--key KEYFILE` adds the detached signature `manifest.json.sig`. The key file
is one line of hex, at least 32 bytes of key material (see `src/sign.rs`;
`Key::generate` makes 32 fresh bytes at mode 0600).

## What `verify` checks

```
harvest verify DIR [--key KEYFILE]
```

Per manifest entry: presence, byte size, sha256, JSONL line count; then the
whole-ledger re-count (`records_ok`); files on disk that the manifest does
not mention are surfaced as `extras` — siblings, **not** violations. The
signature state is one of the strings `valid` / `unsigned` /
`not checked (no --key given)` / `invalid: …`.

## Validation states

`harvest::artifact::classify(&Verification)` maps a report to one state —
the vocabulary the console's Delivery page mirrors:

| state | meaning |
|---|---|
| `verified-unsigned-unchecked` | payload verified; a `.sig` exists but no key was supplied |
| `verified-signed` | payload verified **and** signature validates |
| `verified-unsigned` | payload verified; no signature to check |
| `partial` | a promised file is missing or records failed to recount |
| `tampered` | file present but a byte/size/line promise broke |
| `signature-invalid` | payload intact; the detached signature does not match |
| `missing-manifest` | no readable `manifest.json` (fatal, short-circuits) |

`is_ok()` covers the first three. `partial` outranks `tampered` when both
apply (missing files are the more actionable fact). A wrong key can never
produce `tampered` — payload integrity and signer identity stay separate
verdicts.

## Path safety & budgets

`artifact::load` refuses traversal (`..`) before touching the filesystem,
parses only the documented bounded fields, and errors are the two manual
Display variants `unreadable: …` / `not found: …`. There is no remote
fetching, no auto-repair, and `resume` re-validates existing parts by sha —
it never trusts the folder it is resuming into.

## Trust model, plainly

Browser-side checks (console → Delivery Verify) compare digests you can
recompute independently, but **the CLI is the authority**: it is the only
path that re-counts lines against the manifest and validates the Ed25519
signature with a key that never leaves your machine. An export folder is
evidence only as strong as the transport of the key file; rotate keys with
`harvest sign gen-key` (it refuses to overwrite an existing key).
