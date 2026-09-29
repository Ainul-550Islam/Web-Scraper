// Card — split from ui.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";


export function Card({
  title,
  actions,
  children,
  hover = false,
  foot,
  className = "",
}: {
  title?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  hover?: boolean;
  foot?: ReactNode;
  className?: string;
}) {
  return (
    <section className={`card ${hover ? "card--hover" : ""} ${className}`}>
      {(title || actions) && (
        <header className="card-head">
          <h2 className="t-section">{title}</h2>
          {actions && <span className="spacer" />}
          {actions}
        </header>
      )}
      <div className="card-body">{children}</div>
      {foot && <footer className="card-foot">{foot}</footer>}
    </section>
  );
}
