// ThemeProvider — split from useTheme.tsx; barrel re-exports keep every public path & name

import { useCallback, useEffect, useState, type ReactNode } from "react";
import type { Theme } from "./Theme";
import type { Resolved } from "./Resolved";
import { KEY } from "./KEY";
import { Ctx } from "./Ctx";
import { systemPref } from "./systemPref";


export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(() => {
    try {
      const v = localStorage.getItem(KEY);
      if (v === "dark" || v === "light" || v === "system") return v;
    } catch {
      /* storage disabled — stay in-memory */
    }
    return "dark";
  });
  const resolved: Resolved = theme === "system" ? systemPref() : theme;

  useEffect(() => {
    document.documentElement.dataset["theme"] = resolved;
  }, [resolved]);

  useEffect(() => {
    if (theme !== "system") return;
    const mq = window.matchMedia?.("(prefers-color-scheme: light)");
    const onChange = () =>
      document.documentElement.dataset["theme"] = systemPref();
    mq?.addEventListener("change", onChange);
    return () => mq?.removeEventListener("change", onChange);
  }, [theme]);

  const setTheme = useCallback((t: Theme) => {
    setThemeState(t);
    try {
      localStorage.setItem(KEY, t); // only the theme — verified by test
    } catch {
      /* ignore */
    }
  }, []);

  return <Ctx.Provider value={{ theme, resolved, setTheme }}>{children}</Ctx.Provider>;
}
