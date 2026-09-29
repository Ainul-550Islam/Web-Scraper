// ToastCtx — split from useToasts.tsx; barrel re-exports keep every public path & name

import type { ToastKind } from "./ToastKind";
import type { Toast } from "./Toast";


export interface ToastCtx {
  toasts: Toast[];
  push: (kind: ToastKind, message: string) => void;
  dismiss: (id: number) => void;
}
