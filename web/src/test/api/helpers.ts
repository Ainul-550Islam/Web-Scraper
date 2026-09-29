import { vi } from "vitest";
export function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}
export function neverEndingFetch(): typeof fetch {
  return vi.fn(
    (_input: RequestInfo | URL, init?: RequestInit) =>
      new Promise((_res, rej) => {
        init?.signal?.addEventListener("abort", () =>
          rej(new DOMException("aborted", "AbortError")),
        );
      }),
  ) as unknown as typeof fetch;
}
