### Internal health checks (distinct from `/health`)

The component checks feeding `/diagnostics` classify each subsystem as
`healthy | degraded | unavailable | disabled` — disabled means the config
does not use it (e.g. no redis backend), unavailable means configured but
not currently answerable (e.g. no run attached). Components: `config`
(load + validation state), `runtime` (snapshot age, run status), `storage`
(lanes, bounded `dir_size_bounded` stat walk), `history` (file readable and
parseable), `events_log`, `proxies` (pool sizes + open breakers),
`queue`/`workers` (live handles). These never alter `GET /health`'s fixed
content: probes stay dumb on purpose.
