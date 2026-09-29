// PAGE_LOADERS — split from routes.ts; barrel re-exports keep every public path & name

import { lazy, type ComponentType, type LazyExoticComponent } from "react";


/** Lazy page modules — code-split per feature (vite chunks). */
export const PAGE_LOADERS: Record<string, LazyExoticComponent<ComponentType>> = {
  "/": lazy(() => import("../../features/overview/Overview")),
  "/runs": lazy(() => import("../../features/runs/Runs")),
  "/jobs": lazy(() => import("../../features/jobs/Jobs")),
  "/queue": lazy(() => import("../../features/queue/QueuePage")),
  "/workers": lazy(() => import("../../features/workers/Workers")),
  "/command": lazy(() => import("../../features/command/CommandCenter")),
  "/run": lazy(() => import("../../features/runs/RunDetail")),
  "/events": lazy(() => import("../../features/events/EventsPage")),
  "/delivery": lazy(() => import("../../features/delivery/Delivery")),
  "/config-diff": lazy(() => import("../../features/config/ConfigDiff")),
  "/proxies": lazy(() => import("../../features/proxies/Proxies")),
  "/activity": lazy(() => import("../../features/activity/Activity")),
  "/pipeline": lazy(() => import("../../features/pipeline/Pipeline")),
  "/quality": lazy(() => import("../../features/quality/Quality")),
  "/storage": lazy(() => import("../../features/storage/Storage")),
  "/alerts": lazy(() => import("../../features/alerts/Alerts")),
  "/audit": lazy(() => import("../../features/audit/Audit")),
  "/diagnostics": lazy(() => import("../../features/diagnostics/Diagnostics")),
  "/config": lazy(() => import("../../features/config/ConfigStudio")),
  "/api": lazy(() => import("../../features/api/ApiExplorer")),
  "/export": lazy(() => import("../../pages/ExportCenter")),
  "/crawl": lazy(() => import("../../pages/CrawlStudio")),
  "/ai": lazy(() => import("../../pages/AiAssist")),
  "/control": lazy(() => import("../../pages/ControlJobs")),
  "/control/job": lazy(() => import("../../pages/ControlJobDetail")),
  "/artifacts": lazy(() => import("../../pages/Artifacts")),
};
