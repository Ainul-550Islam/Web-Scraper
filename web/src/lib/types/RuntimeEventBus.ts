// RuntimeEventBus — split from types.ts; barrel re-exports keep every public path & name


export interface RuntimeEventBus {
  dropped: number;
  last_seq: number;
  published: number;
  ring_len: number;
}
