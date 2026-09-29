import { Button, Card, CopyButton, StatusChip } from "../ui";
import { Icon } from "../Icons";
import { navigate } from "../../app/router";
import { useConnection } from "../../lib/connection";
import type { ConfigDocument } from "../../lib/types";

const DEMO_CMD = "cargo run --release -- run --config examples/config.basic.json";

/** First-run orientation — every value below is read from live state, never
 * asserted. If the engine has not answered yet the panel does not render. */
export function WelcomePanel({
  onStartTour,
  onDismiss,
}: {
  onStartTour: () => void;
  onDismiss: () => void;
}) {
  const conn = useConnection();
  const cfg: ConfigDocument | null = conn.config;
  if (!conn.health) return null;
  return (
    <Card
      title={`Welcome to Harvest — ${cfg?.name ?? (conn.health ? "connected engine" : "console")}`}
      actions={<CopyButton text={DEMO_CMD} label="Copy demo command" />}
    >
      <div className="welcome-panel">
        <div className="wp-grid">
          <span className="wp-row">
            <span className="t-label">Backend</span>
            <span className="t-caption"><code className="t-code">{conn.base || "same-origin"}</code></span>
          </span>
          <span className="wp-row">
            <span className="t-label">Health</span>
            <StatusChip status={conn.health.status} label={conn.health.running ? "running" : conn.health.status} />
          </span>
          <span className="wp-row">
            <span className="t-label">Capabilities</span>
            <span className="t-caption">
              {cfg
                ? `${cfg.targets.length} target(s) · ${cfg.proxies.length} proxy egress(es) · robots ${
                    cfg.respect_robots ? "honored" : "off"
                  } · audit ${cfg.audit_path ? "on" : "off"} · attestation ${
                    cfg.attest_key_path ? "keyed" : "unsigned"
                  }`
                : "config not yet loaded — shows after the first /config poll"}
            </span>
          </span>
          <span className="wp-row">
            <span className="t-label">Run the demo</span>
            <code className="t-code">{DEMO_CMD}</code>
          </span>
        </div>
        <div className="wp-actions">
          <Button variant="primary" onClick={() => navigate("/")}>
            Open Dashboard
          </Button>
          <Button onClick={() => navigate("/config")}>
            <Icon.Config size={13} /> Open Configuration
          </Button>
          <Button variant="ghost" onClick={onStartTour}>
            Take the tour (optional)
          </Button>
          <Button variant="ghost" onClick={onDismiss} style={{ marginLeft: "auto" }}>
            Don’t show again <Icon.Close size={11} />
          </Button>
        </div>
      </div>
    </Card>
  );
}
