// TextArea — split from ui.tsx; barrel re-exports keep every public path & name

import { type TextareaHTMLAttributes } from "react";


export function TextArea(props: TextareaHTMLAttributes<HTMLTextAreaElement> & { invalid?: boolean }) {
  const { invalid, ...rest } = props;
  return <textarea {...rest} className="textarea" aria-invalid={invalid || undefined} rows={rest.rows ?? 4} />;
}
