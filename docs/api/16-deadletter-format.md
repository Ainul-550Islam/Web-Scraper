## Dead-letter format

Append+rewrite JSONL at `dlq_path`:
`{"url":"https://…","reason":"blocked:403","attempts":2}`.
Replay with `run --retry-dlq`; successes clear, failures re-enter with
`attempts+1` up to `dlq_max_attempts` (then dead-for-good).
