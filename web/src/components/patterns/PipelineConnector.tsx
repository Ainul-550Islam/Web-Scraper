// PipelineConnector — split from patterns.tsx; barrel re-exports keep every public path & name



export function PipelineConnector() {
  return (
    <span className="pipe-link" aria-hidden="true">
      <svg viewBox="0 0 34 14" preserveAspectRatio="none">
        <path d="M0 7h26m0 0-4-3m4 3-4 3" fill="none" stroke="currentColor" strokeWidth="1.2" />
      </svg>
    </span>
  );
}
