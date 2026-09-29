// toEpoch — split from Runs.tsx; barrel re-exports keep every public path & name


export const toEpoch = (d: string): number | null => (d ? Math.floor(new Date(`${d}T00:00:00Z`).getTime() / 1000) || null : null);
