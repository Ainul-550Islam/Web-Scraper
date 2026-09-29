import { useEffect, useState } from "react";
import { ROUTE_BY_PATH } from "../../app/routes";
import { navigate } from "../../app/router";
import { Button } from "../ui";
import { TOUR_PATHS, markTourDone } from "./TOUR_PATHS";

/**
 * Optional guided tour: a passive corner card that walks the REAL pages.
 * Every step title/description is the actual ROUTES metadata; "Next"
 * navigates for real. It never traps focus, never blocks interaction, and
 * Escape / Skip / Finish dismiss it (and remember the dismissal).
 */
export function ProductTour({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [i, setI] = useState(0);
  useEffect(() => {
    if (open) setI(0);
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        markTourDone();
        onClose();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  if (!open) return null;

  const path = TOUR_PATHS[i]!;
  const def = ROUTE_BY_PATH.get(path);
  const last = i === TOUR_PATHS.length - 1;
  const step = () => {
    if (def && def.path !== currentPath()) navigate(def.path);
  };
  const next = () => {
    if (last) {
      markTourDone();
      onClose();
      return;
    }
    setI((v) => v + 1);
    step();
  };
  const back = () => {
    setI((v) => Math.max(0, v - 1));
    step();
  };
  const skip = () => {
    markTourDone();
    onClose();
  };

  return (
    <aside className="tour-card" role="region" aria-label="Product tour (optional)">
      <div className="tour-steps">
        <span className="t-label">Product tour · {i + 1} / {TOUR_PATHS.length}</span>
        <button className="t-caption" onClick={skip} type="button">
          skip tour
        </button>
      </div>
      <strong>{def?.title ?? path}</strong>
      <p className="t-caption" style={{ margin: 0 }}>
        {def?.desc ?? "Console page"}
      </p>
      <div className="tour-actions">
        {i > 0 && (
          <Button variant="ghost" onClick={back}>
            Back
          </Button>
        )}
        <Button variant="primary" onClick={next}>
          {last ? "Finish" : "Next"}
        </Button>
      </div>
    </aside>
  );
}

function currentPath(): string {
  return window.location.hash.replace(/^#/, "").split("?")[0] || "/";
}
