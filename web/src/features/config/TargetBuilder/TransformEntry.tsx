// TransformEntry — split from TargetBuilder.tsx; barrel re-exports keep every public path & name



export type TransformEntry = { op: string } & Record<string, unknown>;
