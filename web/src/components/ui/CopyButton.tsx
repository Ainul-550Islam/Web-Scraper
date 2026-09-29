// CopyButton — split from ui.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import { Icon } from "../Icons";
import { Button } from "./Button";


export function CopyButton({
  text,
  label = "Copy",
  onCopied,
}: {
  text: string;
  label?: string;
  onCopied?: () => void;
}) {
  const [done, setDone] = useState(false);
  return (
    <Button
      variant="ghost"
      className="sm"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text);
        } catch {
          const ta = document.createElement("textarea");
          ta.value = text;
          document.body.appendChild(ta);
          ta.select();
          document.execCommand("copy");
          ta.remove();
        }
        setDone(true);
        onCopied?.();
        setTimeout(() => setDone(false), 1600);
      }}
    >
      {done ? <Icon.Check size={13} /> : <Icon.Copy size={13} />}
      {done ? "Copied" : label}
    </Button>
  );
}
