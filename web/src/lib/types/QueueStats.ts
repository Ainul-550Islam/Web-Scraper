// QueueStats — split from types.ts; barrel re-exports keep every public path & name


export interface QueueStats {
  depth: number;
  expired: number;
  inflight: number;
  /** 0 = unbounded */
  max_depth: number;
  mode: "fifo";
  per_target_active: [number, number][];
  per_target_depths: [number, number][];
  popped: number;
  pushed: number;
  rejected: number;
}
