// Pure command model shared by the form, route, and validation tests.

export const CONTROL_OPS = ["crawl", "export", "ai"] as const;
export type ControlOp = (typeof CONTROL_OPS)[number];

/* Ceilings mirrored from src/control.rs — keep in step, and say so in the UI. */
export const CEILINGS = {
  maxPages: 1_000_000, // crawl::MAX_PAGES_CEILING
  maxDepth: 20, // crawl::MAX_DEPTH_CEILING
  maxRecords: 5_000_000, // MAX_RECORDS_CEILING
  maxExportBytes: 1_073_741_824, // MAX_EXPORT_BYTES_CEILING (1 GiB)
  maxAiInputBytes: 4_194_304, // MAX_AI_INPUT_CEILING (4 MiB)
  timeoutSecs: 3_600, // MAX_TIMEOUT_SECS
  maxConcurrentJobs: 8, // MAX_CONCURRENT_JOBS
  maxQueueDepth: 64, // MAX_QUEUE_DEPTH
} as const;

export const EXPORT_FORMATS = ["csv", "jsonl", "xlsx", "pdf", "parquet", "xml", "zip"] as const;
export const AI_TASKS = ["extract", "classify", "normalize", "schema-suggest", "label"] as const;
/** The two providers this build registers; there is no way to ask for another. */
export const AI_PROVIDERS = ["deterministic-local", "disabled"] as const;
export const MAX_AI_INLINE_FIELDS = 512;
export const MAX_AI_FIELD_NAME_BYTES = 64;
export const MAX_AI_FIELD_VALUE_BYTES = 4_096;
export const MAX_COMMAND_BODY_BYTES = 65_536;

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).length;
}

export interface CommandLimits {
  max_pages: number;
  max_depth: number;
  max_records: number;
  max_export_bytes: number;
  max_ai_input_bytes: number;
  timeout_secs: number;
}

export interface CrawlDraft {
  targets: string[];
  limits: CommandLimits;
}

export interface ExportDraft {
  formats: string[];
  dataset: string;
  bundle: boolean;
  verify: boolean;
  max_records: number;
  limits: CommandLimits;
}

export interface AiInlineField {
  name: string;
  value: string;
  deterministic: string;
}

export interface AiDraft {
  task: string;
  provider: string;
  confidence: number;
  external_allowed: boolean;
  source_kind: "run_output" | "inline";
  inline_fields: AiInlineField[];
  record_limit: number;
  field_limit: number;
  limits: CommandLimits;
}

export type CommandDraft =
  | { op: "crawl"; crawl: CrawlDraft }
  | { op: "export"; export: ExportDraft }
  | { op: "ai"; ai: AiDraft };

const LIMITS: CommandLimits = {
  max_pages: 100,
  max_depth: 1,
  max_records: 100_000,
  max_export_bytes: 268_435_456,
  max_ai_input_bytes: 1_048_576,
  timeout_secs: 900,
};

export function emptyDraft(op: ControlOp, targetIds: string[] = []): CommandDraft {
  if (op === "crawl") {
    return {
      op: "crawl",
      crawl: {
        targets: targetIds.length === 1 ? [targetIds[0]!] : [],
        limits: { ...LIMITS },
      },
    };
  }
  if (op === "export") {
    return {
      op: "export",
      export: {
        formats: ["csv", "jsonl"],
        dataset: "",
        bundle: false,
        verify: true,
        max_records: 0,
        limits: { ...LIMITS },
      },
    };
  }
  return {
    op: "ai",
    ai: {
      task: "classify",
      provider: "deterministic-local",
      confidence: 0.9,
      external_allowed: false,
      source_kind: "run_output",
      inline_fields: [],
      record_limit: 100,
      field_limit: 32,
      limits: { ...LIMITS },
    },
  };
}

/** The JSON body the route expects — nothing more, nothing renamed. */
export function buildCommand(draft: CommandDraft): Record<string, unknown> {
  if (draft.op === "crawl") {
    return {
      op: "crawl",
      targets: draft.crawl.targets,
      limits: draft.crawl.limits,
    };
  }
  if (draft.op === "export") {
    const e = draft.export;
    return {
      op: "export",
      formats: e.formats,
      dataset: e.dataset.trim() ? e.dataset.trim() : null,
      bundle: e.bundle,
      verify: e.verify,
      max_records: e.max_records > 0 ? e.max_records : null,
      limits: e.limits,
    };
  }
  const a = draft.ai;
  return {
    op: "ai",
    task: a.task,
    provider: a.provider,
    confidence: a.confidence,
    external_allowed: a.external_allowed,
    source:
      a.source_kind === "inline"
        ? {
            kind: "inline",
            fields: a.inline_fields.map((field) => ({
              name: field.name.trim(),
              value: field.value,
              deterministic: field.deterministic.trim() ? field.deterministic.trim() : null,
            })),
          }
        : { kind: "run_output", record_limit: a.record_limit, field_limit: a.field_limit },
    limits: a.limits,
  };
}

export interface CommandIssue {
  field: string;
  message: string;
}

/**
 * Client-side mirror of the server's validation. Anything it catches, the
 * server would have caught too — this only saves the round trip and tells the
 * operator *before* they press submit.
 */
export function validateCommand(draft: CommandDraft, targetIds: string[] = []): CommandIssue[] {
  const issues: CommandIssue[] = [];
  const limits =
    draft.op === "crawl"
      ? draft.crawl.limits
      : draft.op === "export"
        ? draft.export.limits
        : draft.ai.limits;
  const check = (field: string, value: number, ceiling: number, min = 1) => {
    if (!Number.isFinite(value) || value < min) {
      issues.push({ field, message: `${field} must be at least ${min}` });
    } else if (value > ceiling) {
      issues.push({ field, message: `${field} ${value} exceeds the ceiling ${ceiling}` });
    }
  };
  check("limits.max_pages", limits.max_pages, CEILINGS.maxPages);
  check("limits.max_depth", limits.max_depth, CEILINGS.maxDepth, 0);
  check("limits.max_records", limits.max_records, CEILINGS.maxRecords);
  check("limits.max_export_bytes", limits.max_export_bytes, CEILINGS.maxExportBytes);
  check("limits.max_ai_input_bytes", limits.max_ai_input_bytes, CEILINGS.maxAiInputBytes);
  check("limits.timeout_secs", limits.timeout_secs, CEILINGS.timeoutSecs);

  if (draft.op === "crawl") {
    if (draft.crawl.targets.length === 0) {
      issues.push({ field: "targets", message: "pick at least one target" });
    }
    for (const t of draft.crawl.targets) {
      if (targetIds.length > 0 && !targetIds.includes(t)) {
        issues.push({ field: "targets", message: `the attached config declares no target \`${t}\`` });
      }
    }
  }
  if (draft.op === "export" && draft.export.formats.length === 0) {
    issues.push({ field: "formats", message: "pick at least one format" });
  }
  if (draft.op === "ai") {
    if (!Number.isFinite(draft.ai.confidence) || draft.ai.confidence < 0 || draft.ai.confidence > 1) {
      issues.push({ field: "confidence", message: "confidence must be within [0, 1]" });
    }
    if (!AI_TASKS.includes(draft.ai.task as (typeof AI_TASKS)[number])) {
      issues.push({ field: "task", message: "choose a supported AI task" });
    }
    if (!AI_PROVIDERS.includes(draft.ai.provider as (typeof AI_PROVIDERS)[number])) {
      issues.push({ field: "provider", message: "choose a provider registered by this build" });
    }
    if (draft.ai.external_allowed && draft.ai.provider === "disabled") {
      issues.push({
        field: "external_allowed",
        message: "consent has no effect for the disabled provider",
      });
    }
    if (draft.ai.source_kind === "run_output") {
      check("source.record_limit", draft.ai.record_limit, 512);
      check("source.field_limit", draft.ai.field_limit, 512);
    } else {
      const fields = draft.ai.inline_fields;
      if (fields.length === 0) {
        issues.push({ field: "source.fields", message: "add at least one inline field" });
      }
      if (fields.length > MAX_AI_INLINE_FIELDS) {
        issues.push({ field: "source.fields", message: `at most ${MAX_AI_INLINE_FIELDS} inline fields are allowed` });
      }
      for (const [index, field] of fields.entries()) {
        const prefix = `source.fields[${index}]`;
        if (!field.name.trim()) {
          issues.push({ field: `${prefix}.name`, message: "field name must not be empty" });
        }
        if (utf8Length(field.name.trim()) > MAX_AI_FIELD_NAME_BYTES) {
          issues.push({ field: `${prefix}.name`, message: `field name must be at most ${MAX_AI_FIELD_NAME_BYTES} UTF-8 bytes` });
        }
        if (utf8Length(field.value) > MAX_AI_FIELD_VALUE_BYTES) {
          issues.push({ field: `${prefix}.value`, message: `value must be at most ${MAX_AI_FIELD_VALUE_BYTES} UTF-8 bytes` });
        }
        if (field.deterministic.trim() && utf8Length(field.deterministic.trim()) > MAX_AI_FIELD_VALUE_BYTES) {
          issues.push({ field: `${prefix}.deterministic`, message: `deterministic value must be at most ${MAX_AI_FIELD_VALUE_BYTES} UTF-8 bytes` });
        }
      }
      const taskWireName: Record<string, string> = {
        extract: "Extract",
        classify: "Classify",
        normalize: "Normalize",
        "schema-suggest": "SchemaSuggest",
        label: "Label",
      };
      const estimate = new TextEncoder().encode(JSON.stringify({
        task: taskWireName[draft.ai.task] ?? draft.ai.task,
        fields: fields.map((field) => ({
          name: field.name.trim(),
          value: field.value,
          deterministic: field.deterministic.trim() ? field.deterministic.trim() : null,
          redacted: false,
        })),
        declared: [],
      })).length;
      if (estimate > limits.max_ai_input_bytes) {
        issues.push({ field: "source.fields", message: `inline input is approximately ${estimate} bytes, above limits.max_ai_input_bytes (${limits.max_ai_input_bytes})` });
      }
    }
  }
  const bodyBytes = utf8Length(JSON.stringify(buildCommand(draft)));
  if (bodyBytes > MAX_COMMAND_BODY_BYTES) {
    issues.push({
      field: "command",
      message: `request body is ${bodyBytes} bytes, above the server limit of ${MAX_COMMAND_BODY_BYTES}`,
    });
  }
  return issues;
}

