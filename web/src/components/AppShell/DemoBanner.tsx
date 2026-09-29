// DemoBanner — split from AppShell.tsx; barrel re-exports keep every public path & name

import { Icon } from "../Icons";


/**
 * The unmissable DEMO MODE chip (§4). Rendered only when the dev-gated
 * mock layer is active — a production build can never light it because
 * MOCK_ENABLED is a compile-time constant there.
 */
export function DemoBanner({ demo }: { demo: boolean }) {
  if (!demo) return null;
  return (
    <div className="mock-banner" role="status">
      <Icon.Warn size={11} />
      <strong style={{ letterSpacing: "0.06em" }}>DEMO MODE</strong>
      {" "}— deterministic fixtures, not live data · dev build only
    </div>
  );
}
