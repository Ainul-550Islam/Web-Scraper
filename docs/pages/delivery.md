# Delivery Verify

| | |
|---|---|
| Route | `/delivery` |
| Nav group | Data & Guardrails |
| Shortcut | — (none registered) |
| Endpoints | none — file pickers only |
| Polling | none |

In-browser integrity check for an export folder: select the `manifest.json`
plus its payload files; the page re-hashes each file with `crypto.subtle`
(SHA-256) and compares digest and byte size against what the manifest
promised. Result per file: `match`, `mismatch`, `missing`, `no-digest`,
`too-big` (64 MiB hash budget — refused, not skipped).

**The CLI stays the authority**, and the page says so in its own subtitle:
`harvest verify DIR --key KEY` additionally re-counts record lines against
the manifest and validates the detached Ed25519 signature. Neither belongs in
a browser — counting lines client-side means shipping megabytes to WebCrypto
for a result the origin machine computes without moving bytes, and the key
must never leave your machine. This page is a *did the copy survive transit*
check, nothing more.

Notes:

* Nothing is uploaded; files are read via `File` objects in-tab.
* `crypto.subtle` exists only in secure contexts (https / localhost). Served
  over a plain http LAN address the page shows "WebCrypto unavailable here"
  and points at the CLI instead of faking a pass.
* Manifest fields consumed are exactly the documented ones
  (`files[].name/sha256/bytes`, `identity.records`, `identity.generated_ts`)
  — schema owner is `src/artifact.rs`; a manifest without a `files` array is
  rejected as "not a harvest export manifest".
* Files you selected but the manifest doesn't list are tolerated as extras
  and captioned — same semantics as `harvest verify`.
* The verdict vocabulary here mirrors `harvest::artifact::classify`
  (docs/artifact-integrity.md) at the digest level; the state names are
  rendered verbatim so a browser result and a CLI result can be compared by
  eye.

- **Source:** `web/src/features/delivery/Delivery.tsx`
- **Tests:** `web/src/test/final-pages.test.tsx` (match + tamper flows on
  real File objects where WebCrypto exists; honest-unavailable otherwise)
- **See also:** [storage](storage.md) · [docs/artifact-integrity.md](../artifact-integrity.md)
