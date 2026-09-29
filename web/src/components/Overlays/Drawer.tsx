// Drawer — split from Overlays.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";
import { Icon } from "../Icons";
import { useTrap } from "./useTrap";
import { IconButton } from "./IconButton";


export function Drawer({
  title,
  onClose,
  children,
}: {
  title: ReactNode;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useTrap(onClose);
  return (
    <>
      <div className="scrim" onClick={onClose} />
      <aside className="drawer" role="dialog" aria-modal="true" aria-label="details" ref={ref}>
        <header className="drawer-head">
          <div style={{ minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>{title}</div>
          <IconButton label="Close panel" onClick={onClose} style={{ marginLeft: "auto" }}>
            <Icon.Close size={14} />
          </IconButton>
        </header>
        <div className="drawer-body">{children}</div>
      </aside>
    </>
  );
}
