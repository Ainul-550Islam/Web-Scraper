// ApiExplorer — read-only page rendering the hand-verified route manifest
// (web/src/lib/apiRoutes). It fetches nothing: the manifest is the contract,
// pinned against the engine router by tests/route_contract.rs and the web
// contract test. No invented endpoints, no generic Swagger scaffolding.

import { useMemo, useState } from "react";
import { API_ROUTES, type ApiRoute } from "../../../lib/apiRoutes";
import { Card, CopyButton, PageHeader, Segmented, StatusChip } from "../../../components/ui";
import { DataTable } from "../../../components/Table";
import { Page } from "../../../components/AppShell";
import { Icon } from "../../../components/Icons";

const METHOD_TONE: Record<ApiRoute["method"], string> = { GET: "ok", POST: "warn" };

const POST_COUNT = API_ROUTES.filter((r) => r.method === "POST").length;

export default function ApiExplorer() {
  const [kind, setKind] = useState<"all" | "GET" | "POST">("all");
  const rows = useMemo(
    () => (kind === "all" ? API_ROUTES : API_ROUTES.filter((r) => r.method === kind)),
    [kind],
  );

  return (
    <Page>
      <PageHeader
        title="API Explorer"
        sub={
          <>
            {API_ROUTES.length} routes in the admin API · read-mostly ({POST_COUNT} mutations) ·
            every row mirrors <code className="t-code">src/admin.rs</code>
          </>
        }
        actions={
          <CopyButton
            text={JSON.stringify(API_ROUTES, null, 2)}
            label="Copy route manifest"
          />
        }
      />

      <div style={{ display: "flex", gap: 12, alignItems: "center", marginBottom: 12 }}>
        <Segmented
          ariaLabel="method filter"
          value={kind}
          onChange={setKind}
          options={[
            { value: "all", label: `All (${API_ROUTES.length})` },
            { value: "GET", label: `GET (${API_ROUTES.filter((r) => r.method === "GET").length})` },
            { value: "POST", label: `POST (${API_ROUTES.filter((r) => r.method === "POST").length})` },
          ]}
        />
        <span className="t-caption" style={{ marginLeft: "auto" }}>
          auth: bearer token only when <code className="t-code">admin_token</code> is set ·{" "}
          <code className="t-code">/health</code> and <code className="t-code">/live</code> are
          open by design
        </span>
      </div>

      <DataTable<ApiRoute>
        caption="Routes"
        columns={[
          {
            key: "method",
            label: "Method",
            render: (r) => <StatusChip status={METHOD_TONE[r.method] ?? "idle"} dot={false} label={r.method} />,
          },
          {
            key: "path",
            label: "Route",
            render: (r) => <code className="t-code">{r.path}</code>,
          },
          {
            key: "auth",
            label: "Auth",
            render: (r) => (r.auth === "always-open" ? "open" : "bearer if set"),
          },
          { key: "summary", label: "Response / behavior", render: (r) => r.summary },
          {
            key: "params",
            label: "Params",
            hideable: true,
            defaultHidden: false,
            render: (r) => (r.params ? <code className="t-code">{r.params}</code> : <Icon.Close size={10} />),
          },
          {
            key: "doc",
            label: "Doc (repo)",
            hideable: true,
            defaultHidden: false,
            render: (r) =>
              r.doc ? (
                <span className="t-caption">
                  <Icon.Audit size={11} /> docs/api/{r.doc}
                </span>
              ) : (
                <span className="t-caption">—</span>
              ),
          },
        ]}
        rows={rows}
        rowKey={(r) => `${r.method} ${r.path}`}
        emptyTitle="No routes in this filter"
      />

      <div style={{ marginTop: 12 }}>
        <Card title="How this page stays honest">
        <p className="t-caption" style={{ margin: 0, fontSize: 13 }}>
          The manifest lives in <code className="t-code">web/src/lib/apiRoutes/API_ROUTES.ts</code>.
          A Rust integration test (<code className="t-code">tests/route_contract.rs</code>) parses the
          engine router and fails when any <code className="t-code">("METHOD", "/path")</code> arm is
          added, removed, or renamed without updating the manifest; the console test additionally
          proves every path this client calls is in the manifest. So this page can never rot into a
          fictional API.
        </p>
        </Card>
      </div>
    </Page>
  );
}
