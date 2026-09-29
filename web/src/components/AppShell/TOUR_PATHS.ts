// TOUR_PATHS — the product-tour itinerary, defined as REAL routes so the tour
// can only ever show pages that exist. Order follows the buyer's inspection
// flow (connect → inspect → configure → verify), not the nav order.
export const TOUR_PATHS = [
  "/",
  "/config",
  "/runs",
  "/jobs",
  "/workers",
  "/proxies",
  "/queue",
  "/pipeline",
  "/quality",
  "/storage",
  "/alerts",
  "/audit",
  "/diagnostics",
  "/api",
] as const;

/** localStorage key; "done" means the human chose to skip/finish — §32
 * allow-list (safe preferences only, never credentials). */
export const TOUR_KEY = "harvest.console.tour";

/** True once the human finished or skipped the tour (persisted preference). */
export function tourDismissed(): boolean {
  try {
    return localStorage.getItem(TOUR_KEY) === "done";
  } catch {
    return false;
  }
}
export function markTourDone(): void {
  try {
    localStorage.setItem(TOUR_KEY, "done");
  } catch {
    /* storage optional */
  }
}
