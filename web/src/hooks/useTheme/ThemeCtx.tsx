// ThemeCtx — split from useTheme.tsx; barrel re-exports keep every public path & name

import type { Theme } from "./Theme";
import type { Resolved } from "./Resolved";


export interface ThemeCtx {
  theme: Theme;
  resolved: Resolved;
  setTheme: (t: Theme) => void;
}
