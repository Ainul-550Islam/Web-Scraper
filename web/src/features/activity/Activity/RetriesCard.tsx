// RetriesCard — split from Activity.tsx; barrel re-exports keep every public path & name

import { useEffect, useState } from "react";
import { apiGet } from "../../../lib/api";
import { type RetriesResponse } from "../../../lib/types";
import { Card } from "../../../components/ui";


export function RetriesCard() {
  const [data, setData] = useState<RetriesResponse | null>(null);
  useEffect(() => {
    let on = true;
    const go = async () => {
      try {
        const d = await apiGet<RetriesResponse>("/retries");
        if (on) setData(d);
      } catch {
        /* shell handles it */
      }
    };
    void go();
    const h = window.setInterval(go, 6000);
    return () => { on = false; window.clearInterval(h); };
  }, []);
  return (
    <Card title="Retry delays (recent)">
      {data?.recent_delays_ms.length ? (
        <>
          <div className="bars" role="img" aria-label="retry delay samples" style={{ height: 44 }}>
            {data.recent_delays_ms.map((d, i) => (
              <i key={i} style={{ height: `${Math.max(10, Math.min(100, (d / Math.max(...data.recent_delays_ms)) * 100))}%` }} />
            ))}
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>accounting retries</dt><dd>{data.accounting_retries}</dd>
            <dt>lifetime metric</dt><dd>{data.requests_retry}</dd>
          </dl>
        </>
      ) : (
        <p className="t-caption">No retries in the live window — a good default.</p>
      )}
    </Card>
  );
}
