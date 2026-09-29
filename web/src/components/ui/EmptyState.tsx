// EmptyState — split from ui.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";
import { Icon } from "../Icons";


export function EmptyState({
  icon,
  title,
  body,
  action,
}: {
  icon?: ReactNode;
  title: string;
  body?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="state">
      <div className="state-ico">{icon ?? <Icon.Info size={20} />}</div>
      <h3>{title}</h3>
      {body && <p>{body}</p>}
      {action}
    </div>
  );
}
