# Console build → static assets. The runtime stage is plain nginx: the app is
# hash-routed static files (docs/adr/0001), so no SSR, no node server, no
# rewrite rules are needed.
FROM node:24-alpine AS build
WORKDIR /app
# Point the SPA at the same-origin /api proxy when deployed behind nginx
# (ops/docker-compose.yml passes /api). Default empty = engine on the same
# origin, which is how the dev server proxies it anyway.
ARG VITE_API_BASE=
ENV VITE_API_BASE=$VITE_API_BASE
COPY package.json package-lock.json* ./
RUN npm ci
COPY . .
# `npm run build` = tsc --noEmit && vite build; the Dockerfile fails closed if the
# type-check or the dist chunking assertions regress.
RUN npm run build

FROM nginxinc/nginx-unprivileged:stable-alpine AS runtime
COPY --from=build /app/dist /usr/share/nginx/html
COPY nginx.conf.example /etc/nginx/conf.d/default.conf
USER 101:101
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --retries=3 \
  CMD wget -qO- http://127.0.0.1:8080/index.html >/dev/null || exit 1
