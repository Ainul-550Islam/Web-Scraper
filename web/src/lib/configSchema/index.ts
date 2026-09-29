// configSchema: barrel — one file per unit; every prior import path & name still resolves

/**
 * Configuration Studio registry — every control maps to a REAL field in the
 * harvest config model (src/config.rs), verified against a live `GET /config`
 * document. Keys are dotted paths into that document. If a field is not
 * listed in `docs/web-client.md` sources, it does not appear here.
 */
export * from "./ControlKind";
export * from "./ControlDef";
export * from "./ConfigSection";
export * from "./SECTIONS";
export * from "./OUTPUT_KINDS";
export * from "./CONTROLS";
export * from "./CONTROLS_BY_SECTION";
export * from "./getPath";
export * from "./setPath";
export * from "./validateControl";
export * from "./crossFieldErrors";
export * from "./TRANSFORM_OPS";
export * from "./TransformOp";
export * from "./TransformArg";
export * from "./TRANSFORM_ARGS";
