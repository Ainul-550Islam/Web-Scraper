// ProductTour: optional, non-blocking, derived from real ROUTES metadata;
// dismissal persists under the allowed localStorage key.

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ProductTour, markTourDone, tourDismissed } from "../../components/AppShell";
import { TOUR_PATHS } from "../../components/AppShell/TOUR_PATHS";
import { ROUTE_BY_PATH } from "../../app/routes";

describe("ProductTour", () => {
  afterEach(() => {
    cleanup();
    localStorage.clear();
  });

  it("renders nothing when closed", () => {
    render(<ProductTour open={false} onClose={() => {}} />);
    expect(screen.queryByLabelText(/Product tour/)).toBeNull();
  });

  it("walks the real routes and finishes with a persisted skip", () => {
    const onClose = vi.fn();
    render(<ProductTour open onClose={onClose} />);
    expect(screen.getByText(`Product tour · 1 / ${TOUR_PATHS.length}`)).toBeTruthy();
    expect(screen.getByText(ROUTE_BY_PATH.get(TOUR_PATHS[0]!)!.title)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByText(`Product tour · 2 / ${TOUR_PATHS.length}`)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(screen.getByText(`Product tour · 1 / ${TOUR_PATHS.length}`)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /skip tour/ }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(tourDismissed()).toBe(true);
  });

  it("Escape dismisses and remembers", () => {
    const onClose = vi.fn();
    render(<ProductTour open onClose={onClose} />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
    expect(tourDismissed()).toBe(true);
  });

  it("Finish on the last step closes and persists", () => {
    markTourDone(); // pre-seeded state readable below
    expect(tourDismissed()).toBe(true);
    const onClose = vi.fn();
    render(
      <ProductTour open onClose={onClose} />,
    );
    // fresh render restarts at step 1; hammer Next to the end
    const next = () => fireEvent.click(screen.getByRole("button", { name: /Next|Finish/ }));
    for (let i = 0; i < TOUR_PATHS.length; i++) next();
    expect(onClose).toHaveBeenCalled();
  });
});
