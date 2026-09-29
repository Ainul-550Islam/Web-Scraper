// Bundle — split from connection.tsx; barrel re-exports keep every public path & name

import type { AlertsResponse, ConfigDocument, HealthResponse } from "../types";


export interface Bundle {
  health: HealthResponse;
  config: ConfigDocument | null;
  alerts: AlertsResponse | null;
}
