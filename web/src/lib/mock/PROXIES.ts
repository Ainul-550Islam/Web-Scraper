// PROXIES — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { STATS } from "./STATS";

export const PROXIES: T.ProxySnapshot[] = STATS.proxies;
