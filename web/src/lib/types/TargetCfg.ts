// TargetCfg — split from types.ts; barrel re-exports keep every public path & name

import type { ExtractFieldCfg } from "./ExtractFieldCfg";

export interface TargetCfg {
  id: string;
  start_url: string;
  max_depth: number;
  follow_links: boolean;
  link_selector: string | null;
  include: string[];
  exclude: string[];
  max_pages: number;
  mode: string;
  sitemap: boolean;
  sitemap_url: string | null;
  extract: Record<string, ExtractFieldCfg>;
  schema: {
    enforce: "off" | "coerce" | "strict";
    fields: Record<string, string>;
    quarantine_path: string | null;
  };
}
