# Console static hosting reaches the engine through a same-origin /api proxy

Status: **Accepted**

## Context & decision

The built console is origin-agnostic: requests use `VITE_API_BASE` at build
time and a runtime override from the connect screen (credentials never touch
storage, §32). For `ops/docker-compose.yml` the decision is to serve the SPA
from nginx and proxy `/api/` to `harvest:9678` (compose service DNS) — the
browser only ever talks to one origin, so CORS never enters the picture and
the admin token is never carried cross-origin. `web/Dockerfile` therefore
takes `ARG VITE_API_BASE` (default empty = same-origin engine), and
`ops/harvest/console.nginx.conf` is the compose-only variant of
`web/nginx.conf.example` (delta: upstream name + no loopback assumption).

## Consequences

- The console container is pure static + proxy: `read_only`, `cap_drop: ALL`,
  healthchecked via wget on its own index. No node runtime in production.
- Splitting proxy config from the dev-server proxy keeps two failure modes
  visible: dev 404s are the Vite proxy, compose 502s are nginx↔engine DNS —
  each is diagnosed in its own place.
- Contract: `/api/*` on the console origin must always be the admin API
  (the engine's own router answers `/` paths; the proxy strips `/api`).
