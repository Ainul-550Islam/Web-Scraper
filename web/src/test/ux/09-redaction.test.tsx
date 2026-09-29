import { describe, expect, it } from "vitest";
import { maskSecrets } from "../../lib/format";

describe("redaction (§24, §39)", () => {
  it("maskSecrets neutralises bearer, URL credentials and token fields", () => {
    const raw = [
      "Authorization: Bearer abc.def-123/xyz",
      'proxy url "http://user:hunter2@corp-proxy:8080" seen in copy',
      '{"admin_token": "leaky-token", "name": "run-1"}',
    ].join("\n");
    const out = maskSecrets(raw);
    expect(out).not.toContain("abc.def-123");
    expect(out).not.toContain("hunter2");
    expect(out).not.toContain("leaky-token");
    expect(out).toContain("run-1"); // harmless data preserved
  });
});
