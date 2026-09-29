// useToasts — split from useToasts.tsx; barrel re-exports keep every public path & name

import { useContext } from "react";
import type { ToastCtx } from "./ToastCtx";
import { Ctx } from "./Ctx";


export function useToasts(): ToastCtx {
  const c = useContext(Ctx);
  if (!c) throw new Error("useToasts outside ToastProvider");
  return c;
}
