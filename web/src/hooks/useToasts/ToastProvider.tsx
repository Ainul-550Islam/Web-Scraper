// ToastProvider — split from useToasts.tsx; barrel re-exports keep every public path & name

import { useCallback, useRef, useState, type ReactNode } from "react";
import type { ToastKind } from "./ToastKind";
import type { Toast } from "./Toast";
import { Ctx } from "./Ctx";


/**
 * Deliberately quiet: polling refreshes never toast. Only discrete operator
 * events (connected/lost, saved, acknowledged, copied, failed) surface here.
 */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const next = useRef(1);
  const timers = useRef(new Map<number, number>());

  const dismiss = useCallback((id: number) => {
    setToasts((ts) => ts.filter((t) => t.id !== id));
    const h = timers.current.get(id);
    if (h) window.clearTimeout(h);
    timers.current.delete(id);
  }, []);

  const push = useCallback(
    (kind: ToastKind, message: string) => {
      const id = next.current++;
      setToasts((ts) => [...ts.slice(-4), { id, kind, message }]);
      timers.current.set(
        id,
        window.setTimeout(() => dismiss(id), kind === "err" ? 7000 : 4200),
      );
    },
    [dismiss],
  );

  return <Ctx.Provider value={{ toasts, push, dismiss }}>{children}</Ctx.Provider>;
}
