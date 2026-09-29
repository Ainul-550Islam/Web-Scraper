// useHotkeyPalette — split from Overlays.tsx; barrel re-exports keep every public path & name

import { useEffect, useState } from "react";


/** Cmd/Ctrl-K opener; also exposes the open state for the top-bar button. */
export function useHotkeyPalette() {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return { open, setOpen };
}
