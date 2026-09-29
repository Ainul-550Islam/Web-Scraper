/**
 * §10/§49: every exposed Studio control must (a) exist in the real config
 * document, (b) seed with the server's actual value, and (c) use the right
 * input kind. 37 controls ≥ the spec's ~25 — and none are invented.
 */
import { describe, expect, it } from "vitest";
import ConfigStudio from "../features/config/ConfigStudio";
import { CONTROLS, getPath } from "../lib/configSchema";
import { findControl, mountPage } from "./harness";
import config from "./fixtures/config.json";

const doc = config as unknown as Record<string, unknown>;

describe("configuration control coverage", () => {
  it("exposes at least 20 real controls across all sections", () => {
    expect(CONTROLS.length).toBeGreaterThanOrEqual(20);
    expect(CONTROLS.length).toBeGreaterThanOrEqual(25); // registry currently maps 37 real fields
  });

  it("every control path resolves in the live document (no invented fields)", () => {
    for (const c of CONTROLS) {
      expect(getPath(doc, c.path), `path ${c.path}`).toBeDefined();
    }
  });

  it("each non-secret control renders its server value verbatim", async () => {
    const { container } = await mountPage(<ConfigStudio />);
    await findControl(container, "Concurrency"); // settle
    for (const c of CONTROLS.filter((x) => !x.secret)) {
      const raw = getPath(doc, c.path);
      if (c.kind === "toggle") {
        const sw = container.querySelector(`[aria-label="${c.label}"]`);
        expect(sw, c.path).toBeTruthy();
        expect(sw!.getAttribute("aria-checked")).toBe(String(Boolean(raw)));
      } else {
        const input = await findControl(container, c.label);
        expect(input.value, c.path).toBe(raw === null || raw === undefined ? "" : String(raw));
      }
    }
  });

  it("maps kinds to distinct input elements (no sliders, no text-for-everything)", async () => {
    const { container } = await mountPage(<ConfigStudio />);
    await findControl(container, "Concurrency");
    const kinds = new Map(CONTROLS.map((c) => [c.path, c.kind]));
    expect(kinds.get("concurrency")).toBe("number");
    expect(kinds.get("respect_robots")).toBe("toggle");
    expect(kinds.get("output.kind")).toBe("select");
    expect(kinds.get("user_agent")).toBe("text");
    const num = await findControl(container, "Concurrency");
    expect(num.getAttribute("inputmode")).toBe("decimal"); // precise numeric entry — no sliders
    expect(container.querySelectorAll('input[type="range"]').length).toBe(0);
  });

  it("secret controls are flagged and never carry a value into the DOM", async () => {
    const { container } = await mountPage(<ConfigStudio />);
    await findControl(container, "Concurrency");
    const secrets = CONTROLS.filter((c) => c.secret);
    expect(secrets.length).toBeGreaterThanOrEqual(1);
    expect(container.textContent).toContain("SECRET");
    for (const s of secrets) {
      const el = await findControl(container, s.label);
      expect((el as HTMLInputElement).disabled).toBe(true);
      expect(el.value).toBe("<redacted>");
    }
  });
});
