import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import Storage from "../../features/storage/Storage";
import { mountPage, storage } from "../harness";

describe("Storage page", () => {
  it("renders every configured lane from GET /storage", async () => {
    const { getAllByText } = await mountPage(<Storage />);
    for (const lane of Object.keys(storage)) {
      await waitFor(() => expect(getAllByText(new RegExp(lane, "i")).length).toBeGreaterThan(0));
    }
  });

  it("counts configured lanes in the subtitle", async () => {
    const { findByText } = await mountPage(<Storage />);
    const configured = Object.values(storage as Record<string, { configured: boolean }>).filter((l) => l.configured).length;
    expect(await findByText(new RegExp(`${configured} of ${Object.keys(storage).length} lanes configured`))).toBeInTheDocument();
  });
});
