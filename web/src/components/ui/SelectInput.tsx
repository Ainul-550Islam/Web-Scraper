// SelectInput — split from ui.tsx; barrel re-exports keep every public path & name

import { type SelectHTMLAttributes } from "react";


export function SelectInput(
  props: SelectHTMLAttributes<HTMLSelectElement> & { invalid?: boolean },
) {
  const { invalid, children, ...rest } = props;
  return (
    <select {...rest} className="select" aria-invalid={invalid || undefined}>
      {children}
    </select>
  );
}
