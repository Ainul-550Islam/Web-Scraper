# Pipeline

| | |
|---|---|
| Route | `/pipeline` |
| Nav group | Network & Flow |
| Shortcut | — |
| Endpoints | `/pipeline`, `/stats` |
| Polling | 8 s |

Target → transform → enforce → sink rendered with `PipelineNode`/
`PipelineConnector`: one node per declared stage, selection is local state.
Per-target rows carry `enforce ∈ {off, warn, enforce}`, `schema_fields`, and
`transform_ops` — all straight from the engine, no synthesized stages.

- **See also:** [api/08 operational views](../api/08-operational-views-jobs-runs-pipeline.md)
