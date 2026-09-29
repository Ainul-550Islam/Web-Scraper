import type { TargetCfg } from "../../lib/types";
import configFixture from "../fixtures/config.json";
import { mountPage } from "../harness";
export const target0 = (configFixture as unknown as { targets: TargetCfg[] }).targets[0]!;
export async function mountBuilder() {
  const { TargetBuilder } = await import("../../features/config/TargetBuilder");
  let latest: TargetCfg = structuredClone(target0);
  const utils = await mountPage(
    <TargetBuilder target={latest} onChange={(t) => { latest = t; }} onClose={() => undefined} />,
  );
  return { ...utils, latest: () => latest };
}
export const ctrlInput = (container: HTMLElement, label: string) => {
  const ctrl = [...container.querySelectorAll(".ctrl")].find((c) =>
    c.querySelector(".field-label")?.textContent?.trim().startsWith(label),
  );
  const input = ctrl?.querySelector("input,select,textarea");
  if (!input) throw new Error(`no control for "${label}"`);
  return input as HTMLInputElement;
};
