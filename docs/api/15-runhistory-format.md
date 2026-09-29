## Run-history format

Append-only JSONL at `schedule.history_path` (written by `serve`; `run`
writes one line too). Shape as `/recent` above. `history --n 20` renders the
tail; `export --history` uses the newest entry per run to embed the policy
hash.
