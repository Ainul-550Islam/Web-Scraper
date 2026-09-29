import { usePolled } from "../../hooks/usePolled";
export function Poller({ fetcher }: { fetcher: (s: AbortSignal) => Promise<unknown> }) {
  const p = usePolled(fetcher, 1500);
  return <span data-testid="state">{p.state}</span>;
}
