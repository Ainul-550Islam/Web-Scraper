// patterns: barrel — one file per unit; every prior import path & name still resolves

/**
 * patterns.tsx — the remaining design-system primitives from the console spec
 * (§46), each one used at least once by a real page (no dead components):
 *
 *   SearchInput      Activity/Audit filter fields (debounced per §38)
 *   FilterBar        Runs + Audit + Proxies control strips
 *   Tabs             Audit feed switcher (proper role=tablist semantics)
 *   Tooltip          focus-reveal tooltips (Runs policy hashes, table cells)
 *   Checkbox         column-visibility menu (DataTable)
 *   Timeline         run/target lifecycle rails (Runs drawer, Jobs drawer)
 *   ChartCard        framed chart container (Queue per-target pressure)
 *   FormSection      collapsible config sections (Config Studio)
 *   PipelineNode /   shared node graph (Pipeline page + Target Builder strip)
 *   PipelineConnector
 *
 * Plus name-aliases so the library matches the canonical spec vocabulary
 * without duplicating anything.
 */
/* ── canonical-name aliases (single source stays in ui.tsx) ───── */
export {
  TextInput as Input,
  TextArea as Textarea,
  SelectInput as Select,
  StatusChip as StatusBadge,
  HealthCell as HealthIndicator,
} from "../ui";
export * from "./SearchInput";
export * from "./FilterBar";
export * from "./Tabs";
export * from "./Tooltip";
export * from "./Checkbox";
export * from "./TimelineItem";
export * from "./Timeline";
export * from "./ChartCard";
export * from "./FormSection";
export * from "./PipelineNode";
export * from "./PipelineConnector";
