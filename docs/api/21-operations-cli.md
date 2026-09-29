## Operations CLI

The CLI mirrors the admin API for tooling that runs where the server is —
`harvest status|health|jobs|runs|workers|proxies|queue|metrics|datasets`
and `harvest config inspect|explain`. Shared flags: `--json` (pretty) and
`--jsonl` (one object per line — listings stream `items` so `jq` works
without unwrapping envelopes), bounded `--limit` (clamped to the API's
1–200), and `--url HOST:PORT` to target a non-default admin endpoint.
Commands backed by durable state (`status`, `jobs`, `runs`, `config`,
`datasets`) work offline against the config and history files; `workers`,
`queue`, and `metrics` are live-only and **fail closed** — exit 1 naming
the missing/unreachable admin, never a fabricated table. `health` probes
`/health` when an admin address is configured and maps the result onto the
exit code (0 healthy · 1 unreachable/unhealthy), while printing
machine-readable JSON on stdout either way.

`config inspect` prints the same redacted document as `GET /config`
(`<redacted>` markers, `env:VAR` provenance); `config explain --field
FRAGMENT` lists every knob matching the fragment with its type, default,
current value, and doc line — straight from the `CONFIG_KNOBS` table that
the API serves.
