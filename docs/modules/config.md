# Module `config`

Source: [`src/config.rs`](../../src/config.rs) · unit tests: [`src/config/tests.rs`](../../src/config/tests.rs)

Public surface (16 items):

- `struct Config`
  <!-- pub struct Config -->
- `struct AlertCfg`
  <!-- pub struct AlertCfg -->
- `struct TargetSlo`
  <!-- pub struct TargetSlo -->
- `struct ScheduleCfg`
  <!-- pub struct ScheduleCfg -->
- `struct SchedulingCfg`
  <!-- pub struct SchedulingCfg -->
- `struct BreakerOpts`
  <!-- pub struct BreakerOpts -->
- `struct DomainPolicy`
  <!-- pub struct DomainPolicy -->
- `struct ProxyCfg`
  <!-- pub struct ProxyCfg -->
- `struct TargetCfg`
  <!-- pub struct TargetCfg -->
- `struct FieldCfg`
  <!-- pub struct FieldCfg -->
- `enum TransformCfg`
  <!-- pub enum TransformCfg -->
- `enum MaskStrategy`
  <!-- pub enum MaskStrategy -->
- `struct SchemaCfg`
  <!-- pub struct SchemaCfg -->
- `enum SinkCfg`
  <!-- pub enum SinkCfg -->
- `fn redact_credentials` — Remove credentials from a URL for logging: `http://u:p@host:80/x?y=1` becomes `http://u:***@host:80` — once a URL carries credentials we treat the whole tail as sensitive. A URL WITHOUT credentials passes through unchanged (nothing hidden). An unparseable string is reduced to its `scheme://host` origin at best, or `[redacted]`: the string itself may be the secret, so we never echo it back raw.
  <!-- pub fn redact_credentials(s: &str) -> String -->
- `fn redact_url_origin` — Origin only (`scheme://host[:port]`) — for logging webhooks whose secret lives in the path (Slack style) or query.
  <!-- pub fn redact_url_origin(s: &str) -> String -->
