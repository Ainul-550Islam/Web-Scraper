# Proxy Pool

| | |
|---|---|
| Route | `/proxies` |
| Nav group | Network & Flow |
| Shortcut | g p |
| Endpoints | `/proxies`, `/stats` |
| Polling | 6 s |

Egress health with EWMA latency and success rates per proxy, plus the bucket
counters (`healthy + degraded + down + unprobed === total`, pinned by
[contract/proxies.test.ts](../../web/src/test/contract/proxies.test.ts)).
`inflight_denied` surfaces the max-inflight guard. Rotation policy text comes
from `GET /policy` (shown verbatim: proxies fail over for health, *never* to
evade blocks).

- **See also:** [api/11 policy](../api/11-get-policy-static-selfdescription.md)
