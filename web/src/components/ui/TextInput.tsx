// TextInput — split from ui.tsx; barrel re-exports keep every public path & name

import { type InputHTMLAttributes } from "react";


export function TextInput(props: InputHTMLAttributes<HTMLInputElement> & { invalid?: boolean }) {
  const { invalid, ...rest } = props;
  return <input {...rest} className="input" aria-invalid={invalid || undefined} />;
}
