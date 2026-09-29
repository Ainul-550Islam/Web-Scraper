// EventKind — split from types.ts; barrel re-exports keep every public path & name

import type { EVENT_KINDS } from "./EVENT_KINDS";

export type EventKind = (typeof EVENT_KINDS)[number];
