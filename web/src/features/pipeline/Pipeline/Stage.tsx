// Stage — split from Pipeline.tsx; barrel re-exports keep every public path & name

import type { Ctx } from "./Ctx";

export type Stage = {
  id: string;
  title: string;
  source: string;
  meta: (ctx: Ctx) => string[];
};
