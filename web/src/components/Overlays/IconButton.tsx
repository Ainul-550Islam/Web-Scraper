// IconButton — split from Overlays.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";


export function IconButton({ label, onClick, children, style }: {
  label: string; onClick: () => void; children: ReactNode; style?: React.CSSProperties;
}) {
  return (
    <button type="button" className="iconbtn" aria-label={label} onClick={onClick} style={style}>
      {children}
    </button>
  );
}
