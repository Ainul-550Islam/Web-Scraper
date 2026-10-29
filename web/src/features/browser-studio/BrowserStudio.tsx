import { useCallback, useEffect, useMemo, useState } from "react";
import { apiGet, apiPost, commandIdempotencyKey, newCommandIdempotencyNonce } from "../../lib/api";
import { Button, Card, EmptyState, ErrorState, Field, PageHeader, StatusChip, TextInput } from "../../components/ui";
import { Page } from "../../components/AppShell";
import ElementPicker from "./ElementPicker";
import InteractionBuilder, { type StudioAction } from "./InteractionBuilder";
import PreviewTable from "./PreviewTable";
import SelectorEditor, { type SelectorField } from "./SelectorEditor";
import SelectorTree, { findNode } from "./SelectorTree";

interface PageData { page_id: string; session_id: string; final_url: string | null; title: string | null; html: string | null; visible_text: string | null; status: number | null; artifacts: Array<{ id: string; kind: string; sha256: string; bytes: number }> }
interface PreviewResponse { request_id: string; page: PageData; challenge: { detected: boolean; kind: string | null; evidence: string[] }; records: Array<Record<string, unknown>>; artifacts: PageData["artifacts"] }
interface Diagnostics { enabled: boolean; feature_compiled: boolean; runtime_ready: boolean; active_sessions: number; max_sessions: number; engine: string; last_error: string | null }
interface StoredWorkflow { id: string; version: number; workflow: { [key: string]: unknown } }
const emptyTree = { version: 1, root: [] as SelectorField[] };
const newId = () => `field-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

export default function BrowserStudio() {
  const [url, setUrl] = useState("");
  const [sessionId, setSessionId] = useState("");
  const [page, setPage] = useState<PageData | null>(null);
  const [records, setRecords] = useState<Array<Record<string, unknown>>>([]);
  const [tree, setTree] = useState(emptyTree);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [workflow, setWorkflow] = useState<StudioAction[]>([]);
  const [workflowVersion, setWorkflowVersion] = useState(0);
  const [diagnostics, setDiagnostics] = useState<Diagnostics | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const selected = useMemo(() => findNode(tree.root, selectedId), [tree.root, selectedId]);

  const refreshDiagnostics = useCallback(() => {
    apiGet<Diagnostics>("/browser/diagnostics", { noCache: true }).then(setDiagnostics).catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);
  useEffect(() => { refreshDiagnostics(); }, [refreshDiagnostics]);

  const mutate = async <T,>(path: string, body: unknown, op: string): Promise<T> => {
    const nonce = newCommandIdempotencyNonce();
    return apiPost<T>(path, body, { idempotencyKey: commandIdempotencyKey(op, body, nonce) });
  };
  const runPreview = async () => {
    let parsed: URL;
    try { parsed = new URL(url); } catch { setError("Enter a complete http(s) target URL."); return; }
    if (!/^https?:$/.test(parsed.protocol)) { setError("Only http(s) URLs are supported."); return; }
    const id = sessionId || `studio-${Date.now().toString(36)}`;
    const requestId = `preview-${Date.now().toString(36)}`;
    setBusy(true); setError(null); setMessage(null);
    try {
      const result = await mutate<PreviewResponse>("/browser/preview", {
        request_id: requestId, target_id: "browser-studio", target_url: url, session_id: id,
        allowed_hosts: [parsed.hostname], workflow, selector_tree: tree.root.length ? tree : null, capture_screenshot: false,
      }, "browser-preview");
      setSessionId(id); setPage(result.page); setRecords(result.records ?? []);
      setMessage(result.challenge?.detected ? `Challenge detected: ${result.challenge.kind ?? "unknown"}. Policy stops for review.` : `Loaded ${result.page.title ?? result.page.final_url ?? url}`);
      refreshDiagnostics();
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  const testSelectors = async () => {
    if (!sessionId) { setError("Load a target before testing selectors."); return; }
    setBusy(true); setError(null);
    try { const result = await mutate<Array<Record<string, unknown>>>("/browser/selector/test", { session_id: sessionId, selector_tree: tree, max_records: 100 }, "selector-test"); setRecords(result); setMessage(`Selector test returned ${result.length} records.`); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  const inspect = async () => {
    if (!sessionId) return;
    setBusy(true); setError(null);
    try { const result = await mutate<PreviewResponse>("/browser/inspect", { session_id: sessionId, capture_dom: true, capture_screenshot: false }, "browser-inspect"); setPage(result.page); setMessage("Page inspection refreshed."); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  const saveWorkflow = async () => {
    const id = "browser-studio-main";
    setBusy(true); setError(null);
    try { const result = await mutate<StoredWorkflow>("/browser/workflow", { id, version: workflowVersion, workflow, selector_tree: tree.root.length ? tree : null }, "browser-workflow"); setWorkflowVersion(result.version); setMessage(`Workflow saved as ${result.id} v${result.version}.`); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  const addField = (kind: SelectorField["kind"] = "text", selector = "body") => {
    const node: SelectorField = { id: newId(), name: kind === "repeated_record" ? "items" : `field_${tree.root.length + 1}`, kind, selector, required: false, repeat: false, transforms: [], children: kind === "repeated_record" ? [{ id: newId(), name: "title", kind: "text", selector: "h2", required: false, repeat: false, transforms: [], children: [] }] : [] };
    setTree((current) => ({ ...current, root: [...current.root, node] })); setSelectedId(node.id);
  };
  const addChild = () => {
    if (!selectedId) return;
    const child: SelectorField = { id: newId(), name: `child_${Date.now().toString(36)}`, kind: "text", selector: "span", required: false, repeat: false, transforms: [], children: [] };
    const append = (nodes: SelectorField[]): SelectorField[] => nodes.map((node) => node.id === selectedId ? { ...node, children: [...node.children, child] } : { ...node, children: append(node.children) });
    setTree((current) => ({ ...current, root: append(current.root) })); setSelectedId(child.id);
  };
  const replaceNode = (nodes: SelectorField[], id: string, replacement: SelectorField): SelectorField[] => nodes.map((node) => node.id === id ? replacement : { ...node, children: replaceNode(node.children, id, replacement) });
  const removeNode = (nodes: SelectorField[], id: string): SelectorField[] => nodes.filter((node) => node.id !== id).map((node) => ({ ...node, children: removeNode(node.children, id) }));
  const reorderNode = (nodes: SelectorField[], id: string, direction: -1 | 1): SelectorField[] => {
    const at = nodes.findIndex((node) => node.id === id);
    if (at >= 0) {
      const to = Math.max(0, Math.min(nodes.length - 1, at + direction));
      const copy = [...nodes];
      // `noUncheckedIndexedAccess` makes the destructure `SelectorField | undefined`;
      // the element is only spliced back in when it exists.
      const [item] = copy.splice(at, 1);
      if (item) copy.splice(to, 0, item);
      return copy;
    }
    return nodes.map((node) => ({ ...node, children: reorderNode(node.children, id, direction) }));
  };
  const duplicateNode = (nodes: SelectorField[], id: string): SelectorField[] => {
    const out: SelectorField[] = [];
    for (const node of nodes) { out.push(node); if (node.id === id) out.push({ ...node, id: newId(), name: `${node.name}_copy`, children: node.children.map((child) => ({ ...child, id: newId() })) }); else out[out.length - 1] = { ...node, children: duplicateNode(node.children, id) }; }
    return out;
  };
  const updateSelected = (next: SelectorField) => setTree((current) => ({ ...current, root: replaceNode(current.root, next.id, next) }));

  return <Page>
    <PageHeader title="Browser Studio" sub="Authorized browser preview, selector authoring, workflow testing, and bounded diagnostics" />
    {error && <ErrorState error={new Error(error)} onRetry={() => setError(null)} />}
    {message && <p role="status" className="t-caption">{message}</p>}
    <div className="grid grid-2">
      <Card title="Browser runtime" actions={<StatusChip status={diagnostics?.runtime_ready ? "healthy" : diagnostics?.enabled ? "unavailable" : "disabled"} />}>
        {diagnostics ? <dl className="definition-grid"><dt>Feature compiled</dt><dd>{diagnostics.feature_compiled ? "yes" : "no"}</dd><dt>Engine</dt><dd>{diagnostics.engine}</dd><dt>Sessions</dt><dd>{diagnostics.active_sessions} / {diagnostics.max_sessions}</dd><dt>Runtime</dt><dd>{diagnostics.runtime_ready ? "ready" : diagnostics.last_error ?? "not attached"}</dd></dl> : <p className="t-muted">Loading diagnostics…</p>}
        <p className="t-caption t-muted">Browser runs use the configured WebDriver and target scope. Challenges stop for review; this studio does not bypass access controls.</p>
      </Card>
      <Card title="Open authorized target" actions={<Button disabled={busy} onClick={runPreview}>{busy ? "Working…" : "Load / preview"}</Button>}>
        <Field label="Target URL" htmlFor="browser-studio-url" help="Use only a site you are authorized to access. Redirects and workflow navigation are scope-checked by the server.">
          <TextInput id="browser-studio-url" type="url" placeholder="https://example.org/catalog" value={url} onChange={(e) => setUrl(e.target.value)} />
        </Field>
        <Field label="Session ID" htmlFor="browser-studio-session" help="Leave blank to create a new isolated session. Session state remains in process memory unless a secure provider is configured.">
          <TextInput id="browser-studio-session" value={sessionId} onChange={(e) => setSessionId(e.target.value)} placeholder="new isolated session" />
        </Field>
      </Card>
    </div>
    <div className="grid grid-2">
      <Card title="Current page" actions={<Button disabled={!sessionId || busy} onClick={inspect}>Inspect</Button>}>
        {page ? <><p><strong>{page.title || "(untitled page)"}</strong></p><p className="t-caption t-muted">{page.final_url ?? "—"} · HTTP {page.status ?? "unknown"} · session {sessionId}</p><pre className="page-text-preview">{(page.visible_text ?? "").slice(0, 12000) || "No visible text snapshot."}</pre><div className="row-actions"><span>{page.html?.length ?? 0} HTML bytes</span>{page.artifacts?.map((artifact) => <code key={artifact.id}>{artifact.kind}: {artifact.id}</code>)}</div></> : <EmptyState title="No page loaded" body="Load an authorized target to inspect its page state." />}
      </Card>
      <Card title="Element picker">
        <ElementPicker html={page?.html ?? ""} selectedSelector={selected?.selector} onPick={(choice) => { if (selected) updateSelected({ ...selected, selector: choice.selector }); else addField("text", choice.selector); }} />
      </Card>
    </div>
    <Card title="Selector tree" actions={<div className="row-actions"><Button disabled={busy || !sessionId} onClick={testSelectors}>Test selectors</Button><Button disabled={busy} onClick={saveWorkflow}>Save workflow</Button></div>}>
      <div className="grid grid-2"><SelectorTree nodes={tree.root} selectedId={selectedId} onSelect={setSelectedId} onAddField={() => addField()} onAddRecord={() => addField("repeated_record", "article")} onAddChild={addChild} onMove={(id, direction) => setTree((current) => ({ ...current, root: reorderNode(current.root, id, direction) }))} onDuplicate={(id) => setTree((current) => ({ ...current, root: duplicateNode(current.root, id) }))} onDelete={(id) => { setTree((current) => ({ ...current, root: removeNode(current.root, id) })); if (selectedId === id) setSelectedId(null); }} />
        {selected ? <SelectorEditor value={selected} onChange={updateSelected} onTest={testSelectors} busy={busy} /> : <EmptyState title="Select a node to edit" body="Field properties appear here when you select a selector tree node." />}</div>
    </Card>
    <div className="grid grid-2"><Card title="Interaction workflow"><InteractionBuilder steps={workflow} onChange={setWorkflow} /></Card><Card title="Preview records"><PreviewTable records={records} onRefresh={testSelectors} busy={busy} error={null} /></Card></div>
  </Page>;
}
