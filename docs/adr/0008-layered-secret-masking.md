# Secrets are masked engine-side AND redacted in the console

Status: **Accepted**

## Context & decision

Two independent layers: `maskSecrets` runs engine-side before any config export or diagnostics snapshot leaves the process; the console re-redacts `admin_token|password|secret|token|api_key` patterns (plus Bearer headers and URL userinfo) before rendering, copying, or exporting. Defense in depth survives either layer regressing; the copy-report test asserts `<redacted>` lands on the clipboard.
