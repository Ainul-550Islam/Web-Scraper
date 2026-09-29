// ConnectionState — split from connection.tsx; barrel re-exports keep every public path & name

import type { AlertsResponse, ConfigDocument, HealthResponse } from "../types";
import type { usePolled } from "../../hooks/usePolled";

export interface ConnectionState {
  base: string;
  health: HealthResponse | null;
  config: ConfigDocument | null;
  alerts: AlertsResponse | null;
  latencyMs: number | null;
  state: ReturnType<typeof usePolled<unknown>>["state"];
  error: ReturnType<typeof usePolled<unknown>>["error"];
  tokenSet: boolean;
  setToken: (t: string | null) => void;
  refreshAll: () => void;
  updatedAt: number | null;
  intervalSecs: number;
  setIntervalSecs: (n: number) => void;
}
