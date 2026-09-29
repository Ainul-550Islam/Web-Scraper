import { describe, expect, it } from "vitest";
import ConfigStudio from "../../features/config/ConfigStudio";
import { mountPage } from "../harness";

describe("ConfigStudio deep-link", () => {
  it("?target=<id> opens the builder for that target", async () => {
    window.location.hash = "#/config?target=demo-catalog";
    const { findByText } = await mountPage(<ConfigStudio />);
    expect(await findByText(/Target Builder/)).toBeInTheDocument();
    window.location.hash = "";
  });
});
