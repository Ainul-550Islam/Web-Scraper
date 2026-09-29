// Ctx — split from useToasts.tsx; barrel re-exports keep every public path & name

import { createContext } from "react";
import type { ToastCtx } from "./ToastCtx";


export const Ctx = createContext<ToastCtx | null>(null);
