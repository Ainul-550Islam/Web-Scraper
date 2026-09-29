## Politeness audit format (`audit.json`)

```json
{"v":1,"run":"smoke-attest","policy_sha256":"6982dedc0bf3f8940fd52c10425048f41d828cbd9b308955d66d3e9e3bd788b1",
 "user_agent":"harvest-smoke/0.9 (contact: ops@example.com)","respect_robots":true,
 "window":{"start_ts":1789664067,"end_ts":1789664071},
 "hosts":{"127.0.0.1":{
     "requests":5,"ok200":5,"revalidated304":0,"not_found404":0,
     "blocked":{},"transport_errors":0,
     "wait_permits":0,"wait_sleeps":5,"wait_slept_ms":5998,
     "backoff_ms":0,"breaker_abandons":0,"records_written":4,
     "robots":{"fetched":1,"present":true,"denials":1,"crawl_delay_ms":1000},
     "policy":{"requests_per_sec":1.0,"burst":2,"min_interval_ms":500}}},
 "totals":{"requests":5,"ok200":5,"revalidated304":0,"not_found404":0,
     "blocked":0,"transport_errors":0,"wait_sleeps":5,"wait_slept_ms":5998,
     "backoff_ms":0,"records_written":4,"robots_fetched":1,"robots_denials":1}}
```


Property: per host, `requests + robots.fetched` equals the count of your
user-agent's hits in the site's access log (the sample: 5 page/sitemap
requests + 1 robots fetch = 6 hits; sitemap fetches count in `requests`,
robots fetches in their own sub-object). Render: `harvest audit --file … --md`. Real sample:
`examples/output/audit.sample.json` (+ `.sig`).
