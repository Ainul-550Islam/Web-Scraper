// ROUTE_BY_PATH — split from routes.ts; barrel re-exports keep every public path & name

import { ROUTES } from "./ROUTES";


export const ROUTE_BY_PATH = new Map(ROUTES.map((r) => [r.path, r]));
