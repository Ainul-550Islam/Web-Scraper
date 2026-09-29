# Split monolith modules into one file per unit behind barrels

Status: **Accepted**

## Context & decision

A buyer-grade console should be navigable: every component, icon glyph, style layer, schema section, and test suite lives in its own file. Public imports (`../components/ui`, `../lib/mock`, …) keep resolving through directory barrels that re-export every unit, so no consumer import path changed. Units contain the original code verbatim, which keeps review diffs honest.
