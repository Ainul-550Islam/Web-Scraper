// NumberInput — split from ui.tsx; barrel re-exports keep every public path & name

import { useId } from "react";
import type { NumberInputProps } from "./NumberInputProps";

export function NumberInput({ value, onValue, unit, invalid, ...rest }: NumberInputProps) {
  const id = useId();
  return (
    <span className="unit" data-unit={unit}>
      <input
        {...rest}
        id={id}
        className="input"
        inputMode="decimal"
        aria-invalid={invalid || undefined}
        value={value === null ? "" : String(value)}
        onChange={(e) => {
          const t = e.target.value;
          onValue(t === "" ? null : Number(t));
        }}
      />
    </span>
  );
}
