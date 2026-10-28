import { useId } from "react";
import { Button, Field, SelectInput, TextInput } from "../../../components/ui";

export type StudioAction =
  | { action: "click"; selector: string }
  | { action: "input"; selector: string; value: { kind: "literal"; value: string }; clear_first: boolean }
  | { action: "keypress"; selector: string | null; key: string }
  | { action: "wait"; condition: { condition: "selector_visible"; selector: string }; timeout_ms: number }
  | { action: "scroll"; direction: "up" | "down"; pixels: number }
  | { action: "navigate"; url: string }
  | { action: "checkpoint"; name: string; selector: string | null };

export default function InteractionBuilder({ steps, onChange }: { steps: StudioAction[]; onChange: (steps: StudioAction[]) => void }) {
  const id = useId();
  const add = (action: StudioAction["action"]) => {
    const next: StudioAction = action === "click" ? { action, selector: "" }
      : action === "input" ? { action, selector: "", value: { kind: "literal", value: "" }, clear_first: true }
      : action === "keypress" ? { action, selector: null, key: "Enter" }
      : action === "wait" ? { action, condition: { condition: "selector_visible", selector: "" }, timeout_ms: 10000 }
      : action === "scroll" ? { action, direction: "down", pixels: 700 }
      : action === "navigate" ? { action, url: "" }
      : { action: "checkpoint", name: "ready", selector: null };
    onChange([...steps, next]);
  };
  const patch = (index: number, step: StudioAction) => onChange(steps.map((current, i) => i === index ? step : current));
  return <section aria-label="Browser interaction workflow">
    <div className="row-actions"><Button onClick={() => add("click")}>Add click</Button><Button onClick={() => add("input")}>Add input</Button><Button onClick={() => add("keypress")}>Add keypress</Button><Button onClick={() => add("wait")}>Add wait</Button><Button onClick={() => add("scroll")}>Add scroll</Button><Button onClick={() => add("navigate")}>Add navigate</Button><Button onClick={() => add("checkpoint")}>Add checkpoint</Button></div>
    {steps.map((step, index) => <fieldset className="workflow-step" key={`${id}-${index}`}><legend>Step {index + 1}: {step.action}</legend>
      {(step.action === "click" || step.action === "input" || step.action === "wait") && <Field label="CSS selector" htmlFor={`${id}-${index}-selector`}><TextInput id={`${id}-${index}-selector`} value={step.action === "wait" ? step.condition.selector : step.selector} onChange={(e) => patch(index, step.action === "wait" ? { ...step, condition: { ...step.condition, selector: e.target.value } } : { ...step, selector: e.target.value })} /></Field>}
      {step.action === "input" && <Field label="Text value" htmlFor={`${id}-${index}-value`} help="Do not enter passwords or credentials here. Use a secret reference in the saved configuration/API instead."><TextInput id={`${id}-${index}-value`} value={step.value.value} onChange={(e) => patch(index, { ...step, value: { kind: "literal", value: e.target.value } })} /></Field>}
      {step.action === "keypress" && <Field label="Key" htmlFor={`${id}-${index}-key`}><SelectInput id={`${id}-${index}-key`} value={step.key} onChange={(e) => patch(index, { ...step, key: e.target.value })}><option>Enter</option><option>Tab</option><option>Escape</option><option>Backspace</option><option>ArrowDown</option><option>ArrowUp</option></SelectInput></Field>}
      {step.action === "scroll" && <div className="grid grid-2"><Field label="Direction" htmlFor={`${id}-${index}-direction`}><SelectInput id={`${id}-${index}-direction`} value={step.direction} onChange={(e) => patch(index, { ...step, direction: e.target.value as "up" | "down" })}><option value="down">Down</option><option value="up">Up</option></SelectInput></Field><Field label="Pixels" htmlFor={`${id}-${index}-pixels`}><TextInput id={`${id}-${index}-pixels`} type="number" min={1} max={10000} value={step.pixels} onChange={(e) => patch(index, { ...step, pixels: Number(e.target.value) })} /></Field></div>}
      {step.action === "navigate" && <Field label="Authorized URL" htmlFor={`${id}-${index}-url`}><TextInput id={`${id}-${index}-url`} type="url" value={step.url} onChange={(e) => patch(index, { ...step, url: e.target.value })} /></Field>}
      <Button variant="danger" onClick={() => onChange(steps.filter((_, i) => i !== index))}>Remove step</Button>
    </fieldset>)}
  </section>;
}
