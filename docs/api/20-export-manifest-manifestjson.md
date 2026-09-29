## Export manifest (`manifest.json`)

```json
{"v":1,"generated_ts":1789664071,"source_files":["/tmp/hattest/out.jsonl"],
 "split_target":false,"records":4,"unique_urls":4,"skipped_lines":0,
 "runs":["smoke-attest"],"targets":{"shop":4},
 "columns":["run","target","url","depth","ts","fingerprint","egress","price","title"],
 "files":[{"name":"records.csv","sha256":"6c8a05ce…","bytes":671,"lines":5},
          {"name":"records.jsonl","sha256":"78bcdb26…","bytes":1016,"lines":4}]}
```

With `--history`, a `"policy_sha256":{"<run>":"<sha>"}` field is added for
each run found in the history file — the same value `/recent` reports.

Deterministic: same inputs ⇒ byte-identical files (only `generated_ts`
moves). Signature (with `--key`): `manifest.json.sig` holds one hex
HMAC-SHA256 over the manifest bytes; the manifest's `files[].sha256` chain
covers every payload file.
