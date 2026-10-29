import { useMemo, useState } from "react";
import { Button, EmptyState, TextInput } from "../../components/ui";

export interface ElementChoice { selector: string; tag: string; text: string; path: string }

function cssEscape(value: string): string {
  if (globalThis.CSS?.escape) return globalThis.CSS.escape(value);
  return value.replace(/[^a-zA-Z0-9_-]/g, (c) => `\\${c.codePointAt(0)?.toString(16) ?? "20"} `);
}
function selectorFor(element: Element): string {
  if (element.id) return `#${cssEscape(element.id)}`;
  const classes = [...element.classList].filter(Boolean).slice(0, 3);
  if (classes.length) return `${element.tagName.toLowerCase()}${classes.map((name) => `.${cssEscape(name)}`).join("")}`;
  const segments: string[] = [];
  let current: Element | null = element;
  while (current && current.tagName.toLowerCase() !== "html" && segments.length < 6) {
    const tag = current.tagName.toLowerCase();
    const siblings = current.parentElement ? [...current.parentElement.children].filter((child) => child.tagName === current?.tagName) : [];
    const position = siblings.length > 1 ? `:nth-of-type(${siblings.indexOf(current) + 1})` : "";
    segments.unshift(`${tag}${position}`);
    current = current.parentElement;
  }
  return segments.join(" > ");
}

export default function ElementPicker({ html, onPick, selectedSelector = "" }: {
  html: string;
  onPick: (choice: ElementChoice) => void;
  selectedSelector?: string;
}) {
  const [filter, setFilter] = useState("");
  const choices = useMemo(() => {
    if (!html) return [];
    const parsed = new DOMParser().parseFromString(html, "text/html");
    return [...parsed.body.querySelectorAll("*")].slice(0, 500).map((element): ElementChoice => ({
      selector: selectorFor(element),
      tag: element.tagName.toLowerCase(),
      text: (element.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 160),
      path: selectorFor(element),
    })).filter((entry) => !["script", "style", "meta", "link", "noscript"].includes(entry.tag));
  }, [html]);
  const shown = choices.filter((choice) => `${choice.tag} ${choice.selector} ${choice.text}`.toLowerCase().includes(filter.toLowerCase())).slice(0, 80);
  return (
    <section aria-label="Page element picker">
      <div className="grid grid-2">
        <TextInput aria-label="Filter page elements" placeholder="Filter by tag, selector, or text" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <span className="t-caption t-muted">{choices.length} DOM elements · showing at most 80</span>
      </div>
      {!html ? <EmptyState title="No page snapshot" body="Load an authorized target in Browser Studio first." /> : shown.length === 0 ? <EmptyState title="No matching elements" body="Try another tag, CSS class, or text fragment." /> : (
        <div className="table-wrap" style={{ maxHeight: 360, overflow: "auto" }}>
          <table className="data"><thead><tr><th>element</th><th>selector candidate</th><th>sample text</th><th>action</th></tr></thead>
            <tbody>{shown.map((choice, index) => <tr key={`${choice.selector}-${index}`}>
              <td><code>&lt;{choice.tag}&gt;</code></td><td><code>{choice.selector}</code></td><td className="t-muted">{choice.text || "—"}</td>
              <td><Button variant={selectedSelector === choice.selector ? "primary" : "default"} onClick={() => onPick(choice)} aria-label={`Select ${choice.selector}`}>Select</Button></td>
            </tr>)}</tbody>
          </table>
        </div>
      )}
    </section>
  );
}
