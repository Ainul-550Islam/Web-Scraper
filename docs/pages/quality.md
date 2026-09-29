# Data Quality

| | |
|---|---|
| Route | `/quality` |
| Nav group | Network & Flow |
| Shortcut | — |
| Endpoints | `/stats`, `/pipeline` |
| Polling | 8 s |

Extracted → written with every loss itemized (quarantined, schema drifts,
deduped). Numbers only where the engine counts them; the run-level summary shape
is the same object the CLI prints, so web and CLI can never disagree.

- **See also:** [api/14 record format](../api/14-record-format-jsonl-sink-and-the-input-c.md)
