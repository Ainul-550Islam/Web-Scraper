import { Component, type ReactNode } from "react";

/** Keep a failed page/chunk from turning the whole operator console blank.
 * Error objects can contain endpoints/data: never echo their raw text or stack.
 */
export class PageErrorBoundary extends Component<
  { children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <section role="alert" className="card" style={{ padding: 24 }}>
        <h2>This page could not be loaded</h2>
        <p>Choose another page from the navigation, or reload the console. Reloading will clear this tab’s memory-only token and unsaved drafts.</p>
        <button className="btn" type="button" onClick={() => window.location.reload()}>
          Reload page
        </button>
      </section>
    );
  }
}
