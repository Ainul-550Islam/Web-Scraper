/**
 * KeyReveal (features/managed/keys/KeyReveal.tsx).
 *
 * The plaintext is the one value the API hands out that must not survive this
 * render. These tests pin that: shown once, copyable, and gone the moment the
 * dialog closes — with nothing written to storage or left in the DOM.
 */
import { describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { fireEvent, screen } from "@testing-library/react";
import { KeyReveal } from "../../features/managed/keys/KeyReveal";
import { mountManaged } from "./harness";

const SECRET = "hv_live_supersecret";

function storageDump(): Record<string, string> {
  const out: Record<string, string> = {};
  for (let i = 0; i < localStorage.length; i += 1) {
    const k = localStorage.key(i);
    if (k) out[k] = localStorage.getItem(k) ?? "";
  }
  for (let i = 0; i < sessionStorage.length; i += 1) {
    const k = sessionStorage.key(i);
    if (k) out[`session:${k}`] = sessionStorage.getItem(k) ?? "";
  }
  return out;
}

function OpenWrapper() {
  const [open, setOpen] = useState(true);
  if (!open) return null;
  return (
    <KeyReveal
      apiKey={SECRET}
      keyId="key-new"
      title="Key created"
      onClose={() => setOpen(false)}
    />
  );
}

async function mountOpen() {
  return mountManaged(<OpenWrapper />, {});
}

async function mount(keyId: string) {
  return mountManaged(
    <KeyReveal apiKey={SECRET} keyId={keyId} title="Key created" onClose={() => undefined} />,
    {},
  );
}

describe("KeyReveal", () => {
  it("shows the plaintext once, in a dialog, with no other copy in the document", async () => {
    const before = storageDump();
    const { container } = await mount("key-new");

    expect(await screen.findByTestId("revealed-key")).toHaveTextContent(SECRET);
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    // Exactly one element carries the secret, and it lives in the modal.
    expect(container.querySelectorAll('[data-testid="revealed-key"]')).toHaveLength(1);
    expect(container.innerHTML.split(SECRET).length - 1).toBe(1);

    // Nothing was persisted by rendering the secret.
    const after = storageDump();
    expect(Object.keys(after).length).toBe(Object.keys(before).length);
    expect(JSON.stringify(after)).not.toContain(SECRET);
  });

  it("copies the plaintext and confirms it", async () => {
    // jsdom has no Clipboard API; give it one so the component's happy path runs.
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { getByRole } = await mount("key-new");
    fireEvent.click(getByRole("button", { name: "Copy key" }));
    expect(writeText).toHaveBeenCalledWith(SECRET);
    expect(await screen.findByRole("status")).toHaveTextContent(/Copied/);
  });

  it("removes the plaintext from the document when the dialog is dismissed", async () => {
    // The page owns the visibility; this mirrors how ApiKeys renders it.
    const { getByRole, container } = await mountOpen();
    await screen.findByTestId("revealed-key");
    fireEvent.click(getByRole("button", { name: "Close dialog" }));
    expect(await screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(container.innerHTML).not.toContain(SECRET);
    expect(screen.queryByTestId("revealed-key")).not.toBeInTheDocument();
  });

  it("labels the dialog and names the key it belongs to", async () => {
    const { findByText } = await mount("key-rotated-7");
    expect(await findByText("Key created")).toBeInTheDocument();
    expect(screen.getByText(/Key id key-rotated-7/)).toBeInTheDocument();
  });
});
