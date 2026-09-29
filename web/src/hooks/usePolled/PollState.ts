// PollState — split from usePolled.ts; barrel re-exports keep every public path & name


export type PollState = "connecting" | "live" | "refreshing" | "paused" | "offline";
