// RouteDef — split from routes.ts; barrel re-exports keep every public path & name

import { type IconName } from "../../components/Icons";

export interface RouteDef {
  path: string;
  label: string;
  icon: IconName;
  group: "Operations" | "Network & Flow" | "Data & Guardrails" | "Console" | "Delivery";
  title: string;
  desc?: string;
  badge?: "alerts" | "queue";
  disabled?: boolean; // grayed until an API exists — none today
}
