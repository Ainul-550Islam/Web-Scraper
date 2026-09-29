// Toast — split from useToasts.tsx; barrel re-exports keep every public path & name

import type { ToastKind } from "./ToastKind";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}
