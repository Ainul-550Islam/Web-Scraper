## Revalidation store format

`revalidate_store` JSONL: per URL `{"etag":…,"lm":…,"links":[…],"ts":…}`
(keys `etag`/`lm`/`links` omitted when empty; `links` capped at 500).
`ts` marks the last confirmed-fresh (200/304) event and drives
`revalidate_ttl_secs` expiry.
