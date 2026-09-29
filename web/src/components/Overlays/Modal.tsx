// Modal — split from Overlays.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";
import { Icon } from "../Icons";
import { useTrap } from "./useTrap";
import { IconButton } from "./IconButton";


export function Modal({
  title,
  onClose,
  children,
  footer,
  labelledBy,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  labelledBy?: string;
}) {
  const ref = useTrap(onClose);
  return (
    <>
      <div className="scrim" onClick={onClose} />
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy ?? "modal-title"}
        ref={ref}
      >
        <div className="drawer-head">
          <h2 id="modal-title" className="t-section">{title}</h2>
          <IconButton label="Close dialog" onClick={onClose} style={{ marginLeft: "auto" }}>
            <Icon.Close size={14} />
          </IconButton>
        </div>
        <div style={{ padding: 16 }}>{children}</div>
        {footer && (
          <div className="card-foot" style={{ justifyContent: "flex-end" }}>{footer}</div>
        )}
      </div>
    </>
  );
}
