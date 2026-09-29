// types: barrel — one file per unit; every prior import path & name still resolves

/**
 * TypeScript mirror of the harvest admin API surface.
 * Every shape here was captured from a live server (docs/web-client.md
 * explains the derivation); nothing is speculative.
 */
/* ── health / stats ─────────────────────────────────────────── */
export * from "./HealthResponse";
export * from "./LiveResponse";
export * from "./MetricsCounters";
export * from "./ProxySnapshot";
export * from "./StatsResponse";
export * from "./RuntimeAccounting";
export * from "./RecentFailure";
export * from "./RuntimeEventBus";
export * from "./QueueStats";
export * from "./WorkerStats";
export * from "./WorkerInfo";
export * from "./RuntimeSnapshot";
export * from "./RuntimeResponse";
export * from "./isRuntimeSnapshot";
export * from "./ComponentStatus";
export * from "./WorkerHealth";
export * from "./QueueHealth";
export * from "./JobTally";
export * from "./JobsResponse";
export * from "./RunInfo";
export * from "./Paged";
export * from "./RunsResponse";
export * from "./RunsQuery";
export * from "./RecentRunRow";
export * from "./EVENT_KINDS";
export * from "./EventKind";
export * from "./EngineEvent";
export * from "./EventsResponse";
export * from "./FailuresResponse";
export * from "./RetriesResponse";
export * from "./PipelineTarget";
export * from "./PipelineResponse";
export * from "./StorageLane";
export * from "./StorageResponse";
export * from "./DiagStatus";
export * from "./DiagComponent";
export * from "./DiagnosticsResponse";
export * from "./AlertItem";
export * from "./AlertsResponse";
export * from "./OpsEntry";
export * from "./OpsResponse";
export * from "./AuditEntry";
export * from "./AuditResponse";
export * from "./ExtractFieldCfg";
export * from "./TargetCfg";
export * from "./ConfigDocument";
export * from "./AckRequest";
export * from "./NoteRequest";
