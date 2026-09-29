// NumberInputProps — split from ui.tsx; barrel re-exports keep every public path & name

import { type InputHTMLAttributes } from "react";


export interface NumberInputProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "onChange" | "value"> {
  value: number | null;
  onValue: (n: number | null) => void;
  unit?: string;
  invalid?: boolean;
}
