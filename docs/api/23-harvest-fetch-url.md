## `harvest fetch <url>`

One-shot GET for egress checks. Prints
`<status> <url> <bytes> ct=<content-type> egress=<proxy-id|direct>`. Goes
through the real fetcher stack (timeouts, body cap, redirect cap; robots is
not consulted) but on **built-in defaults** — it takes no `--config`, so
pacing/policy config doesn't apply — and with no `proxies` in defaults, it
always egresses `direct`; for proxy behavior use a one-page crawl config.
