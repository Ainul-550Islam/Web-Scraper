## Checkpoint format

`ckpt_path` holds one JSON object (atomic tmp+rename):

```json
{"v":1,"run":"demo-run","policy_sha256":"…","saved_ts":1789663966,
 "global_pages":3,
 "pending":[{"url":"http://127.0.0.1:8099/product/2","depth":1,"target":0}],
 "targets":[{"id":"demo-catalog","visited":["http://127.0.0.1:8099/index.html"],"fp":["14a30188…"],
              "written":3}]}
```

Loaded only with `--resume`; `policy_sha256` mismatch ⇒ refusal (exit 1),
never silent resume under changed rules. Deleted on completion.
