## Record format (JSONL sink, and the input contract of `export`)

One line per written record — meta fields first, then extracted `fields`:

```json
{"run":"smoke-attest","target":"shop","url":"http://127.0.0.1:8099/product/1",
 "depth":0,"ts":1789664069,
 "fingerprint":"2969c0a5198f317a5d93ffe68225d9224cc5dbc10b23ffead596a52399be828c",
 "egress":"direct",
 "fields":{"price":"$19.99","title":"Widget One"}}
```

(a verbatim line from `examples/output/records.sample.jsonl`; `fingerprint`
is the sha256 of the normalized visible text, so identical content
re-crawled re-produces it).

`egress` is the proxy id used (`"direct"` when no proxy). `ts` is unix
seconds. See `examples/output/records.sample.jsonl`.
