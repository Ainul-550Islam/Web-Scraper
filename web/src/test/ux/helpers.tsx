export function seededRuns(n = 22) {
  return {
    page: {
      items: Array.from({ length: n }, (_, i) => ({
        config_sha256: `sha${i}`.padEnd(64, "0"),
        duration_ms: 1000 + i,
        error: null,
        index: i,
        quarantined: 0,
        run: "demo-run",
        status: i % 5 === 0 ? "error" : "ok",
        ts: 1_789_963_000 + i,
        written: n - i,
      })),
      limit: 15,
      offset: 0,
      total: n,
    },
    window: n,
    window_note: "listings scan the newest 512 history lines; `total` is within that window",
  };
}
