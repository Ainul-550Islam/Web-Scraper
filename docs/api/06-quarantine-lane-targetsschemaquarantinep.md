### Quarantine lane (`targets[].schema.quarantine_path`)

When a target enforces `strict`, records that violate the declared schema
never reach the sink; each is appended (one per line) to its own JSONL:

```json
{"url":"http://x/p/1","depth":1,"ts":"2026-09-20T10:00:00Z",
 "reasons":["price: parse_number: no number in `free!`"],
 "fields":{"price":"free!"}}
```

`reasons` covers BOTH layers: transform-step failures
(`<field>: <step>: <why>`) and type-check failures coercion cannot fix.
`harvest_records_quarantined` counts them; `/events` carries one
`record_quarantined` per record. The run itself still exits 0.
