# Poll with per-page intervals instead of SSE or WebSocket

Status: **Accepted**

## Context & decision

The admin API's polling GET routes return bounded JSON snapshots; the API also has explicit operator and bounded control-plane POST routes. There is no SSE/WebSocket event stream to subscribe to, and adding one would mean inventing backend surface the spec forbids. Each page declares its own interval (Overview 5s, audit 10s, …) through `usePolled`, which pauses while the tab is hidden, backs off one extra interval on HTTP 429, and revalidates on `visibilitychange`. Server load stays bounded and the failure model stays simple.
