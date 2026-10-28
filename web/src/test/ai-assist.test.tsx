/**
 * AI Assist: the page must never present an AI result as validated, never
 * invent one, and never expose a value the engine would have redacted.
 */
import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import AiAssist, { providerStatus } from "../pages/AiAssist";
import AiSuggestionCard, { bandOf, provenanceLabel } from "../components/AiSuggestionCard";
import { mountPage } from "./harness";
import { REDACTION_MARK } from "../lib/redaction";

function file(name: string, body: unknown): File {
  return new File([typeof body === "string" ? body : JSON.stringify(body)], name, { type: "application/json" });
}

const PROVIDER_ON = {
  enabled: true,
  external_allowed: false,
  provider: "deterministic-local",
  model: "shape-inference-1",
  request_budget: 100,
  input_budget_bytes: 1048576,
  thresholds: { accept: 0.9, review: 0.6 },
  overwrite_validated: false,
};

const PROVIDER_OFF = { enabled: false, provider: "disabled" };

const PROVIDER_EXTERNAL = {
  enabled: true,
  external_allowed: false,
  provider: "openai",
  model: "gpt-placeholder",
  request_budget: 10,
};

function suggestion(field: string, confidence: number, provenance = "ai-suggestion") {
  return {
    field,
    value: "Widget",
    suggested: "text",
    confidence,
    rationale: "shape inference",
    provenance,
  };
}

async function loadBoth(provider: { provider?: string }, outcome: unknown, expectedCards = 0) {
  const utils = await mountPage(<AiAssist />);
  fireEvent.change(utils.container.querySelector('input[aria-label="provider document"]') as HTMLInputElement, {
    target: { files: [file("ai.json", provider)] },
  });
  // Wait for the *document's* provider name to reach the facts list — generic
  // strings like "disabled by default" are on screen from the first render.
  await waitFor(() => expect(utils.container.textContent).toContain(provider.provider ?? "disabled"));
  if (outcome !== null) {
    fireEvent.change(utils.container.querySelector('input[aria-label="suggestion document"]') as HTMLInputElement, {
      target: { files: [file("outcome.json", outcome)] },
    });
    await waitFor(() =>
      expect(
        utils.container.querySelectorAll(".ai-card").length > 0 ||
          (utils.container.textContent ?? "").includes("No suggestions in this document"),
      ).toBe(true),
    );
    if (expectedCards > 0) {
      await waitFor(() => expect(utils.container.querySelectorAll(".ai-card").length).toBe(expectedCards));
    }
  }
  return utils;
}

describe("AI Assist — provider state", () => {
  it("is disabled by default and says so, with no suggestions", async () => {
    const { container, findByText } = await mountPage(<AiAssist />);
    expect(await findByText("No provider configured")).toBeTruthy();
    expect(container.textContent).toContain("disabled by default");
    expect(container.textContent).not.toContain("AI SUGGESTION");
  });

  it("treats an unconfigured provider document as disabled", async () => {
    const { container } = await loadBoth(PROVIDER_OFF, null);
    expect(container.textContent).toContain("AI disabled — no suggestions");
    expect(container.textContent).not.toContain("AI SUGGESTION");
  });

  it("blocks an external provider without consent instead of pretending it works", async () => {
    const { container } = await loadBoth(PROVIDER_EXTERNAL, null);
    expect(container.textContent).toContain("blocked — no consent");
    expect(container.textContent).toContain("external_allowed");
    expect(container.textContent).toMatch(/refuses to send data/);
  });

  it("rejects a malformed provider document loudly", async () => {
    const { container, findByText } = await mountPage(<AiAssist />);
    fireEvent.change(container.querySelector('input[aria-label="provider document"]') as HTMLInputElement, {
      target: { files: [file("bad.json", '{"nope":1}')] },
    });
    expect(await findByText(/provider document rejected/)).toBeTruthy();
  });

  it("derives status from the document without calling any endpoint", async () => {
    const status = providerStatus(PROVIDER_ON, 3);
    expect(status.enabled).toBe(true);
    expect(status.configured).toBe(true);
    expect(status.external).toBe(false);
    expect(status.budget).toEqual({ used: 3, max: 100 });
    const off = providerStatus(null);
    expect(off.enabled).toBe(false);
    expect(off.name).toBe("disabled");
  });
});

describe("AI Assist — suggestions", () => {
  it("labels every AI suggestion as AI SUGGESTION, never as validated", async () => {
    const { container } = await loadBoth(
      PROVIDER_ON,
      {
        state: "ok",
        suggestions: [suggestion("title", 0.95), suggestion("price", 0.72), suggestion("note", 0.2)],
        requests_used: 3,
      },
      3,
    );
    const badges = [...container.querySelectorAll('[data-testid="provenance-label"]')].map((n) => n.textContent);
    expect(badges.length).toBe(3);
    expect(new Set(badges)).toEqual(new Set(["AI SUGGESTION"]));
    expect(container.textContent).not.toContain("VALIDATED\n");
  });

  it("shows the confidence bands the stage reported", async () => {
    const { container } = await loadBoth(
      PROVIDER_ON,
      { state: "ok", suggestions: [suggestion("a", 0.95), suggestion("b", 0.72), suggestion("c", 0.2)] },
      3,
    );
    const cards = [...container.querySelectorAll(".ai-card")] as HTMLElement[];
    expect(cards.map((c) => c.dataset.band)).toEqual(["high", "medium", "low"]);
    expect(container.textContent).toContain("1 high · 1 medium · 1 low");
  });

  it("keeps a deterministic value labelled VALIDATED", async () => {
    const { container } = await mountPage(
      <AiSuggestionCard suggestion={suggestion("url", 1, "deterministic")} />,
    );
    expect(container.querySelector('[data-testid="provenance-label"]')?.textContent).toBe("VALIDATED");
    expect(container.querySelector(".ai-card")?.className).not.toContain("ai-assisted");
  });

  it("moves a suggestion through review states only when a human acts", async () => {
    const { container } = await loadBoth(PROVIDER_ON, { state: "ok", suggestions: [suggestion("title", 0.95)] }, 1);
    expect(container.textContent).toContain("review: pending");
    const accept = [...container.querySelectorAll("button")].find((b) => b.textContent === "Accept");
    fireEvent.click(accept as HTMLButtonElement);
    await waitFor(() => expect(container.textContent).toContain("review: accepted"));
  });

  it("reports a failed stage as failure, never as an empty success", async () => {
    const { container } = await loadBoth(PROVIDER_ON, { state: "failed", suggestions: [] });
    await waitFor(() => expect(container.textContent).toContain("Stage state: failed"));
    expect(container.textContent).toContain("No suggestions in this document");
  });

  it("redacts credential-shaped field values on the card", async () => {
    const { container } = await mountPage(
      <AiSuggestionCard suggestion={suggestion("api_key", 0.9)} />,
    );
    expect(container.textContent).toContain(REDACTION_MARK);
  });

  it("warns when a suggestion would replace a validated value", async () => {
    const { container } = await mountPage(
      <AiSuggestionCard suggestion={{ ...suggestion("price", 0.99), replaces_validated: true }} />,
    );
    expect(container.textContent).toMatch(/Would replace an existing deterministic value/);
  });

  it("bands confidence at the documented thresholds", () => {
    expect(bandOf(0.95)).toBe("high");
    expect(bandOf(0.9)).toBe("high");
    expect(bandOf(0.6)).toBe("medium");
    expect(bandOf(0.59)).toBe("low");
    expect(bandOf(Number.NaN)).toBe("low");
    expect(provenanceLabel("ai-suggestion")).toBe("AI SUGGESTION");
    expect(provenanceLabel("deterministic")).toBe("VALIDATED");
  });
});
