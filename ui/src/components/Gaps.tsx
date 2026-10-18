// Your typical numbers against the benchmark tier, worst first.
import { formatMetric, pctTone } from "../lib/format";
import type { GapsView } from "../types";

export function GapsTable({ view, limit }: { view: GapsView; limit?: number }) {
  const rows = limit ? view.gaps.slice(0, limit) : view.gaps;
  if (!view.reference) {
    return <p className="muted">Not enough higher-rank games collected yet for this role.</p>;
  }
  return (
    <>
      <table className="gaps">
        <thead>
          <tr>
            <th>Metric</th>
            <th className="num">Yours</th>
            <th className="num">Their median</th>
            <th>Where you'd rank among them</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((g) => {
            const p = Math.round(g.better_than_pct);
            // Medians of counts can land on .5; whole numbers keep the metric's precision.
            const fmt = (v: number) =>
              formatMetric(v, g.unit, Number.isInteger(v) ? g.decimals : Math.max(g.decimals, 1));
            return (
              <tr key={g.metric}>
                <td>
                  {g.label}
                  {g.early && (
                    <span className="tag" title="Measured by 15:00: the most actionable numbers">
                      early
                    </span>
                  )}
                </td>
                <td className="num strong">{fmt(g.yours)}</td>
                <td className="num muted">{fmt(g.benchmark_median)}</td>
                <td>
                  <div className="pct" title={`${g.n} benchmark players`}>
                    <div className={`pct-bar ${pctTone(p)}`}>
                      <span style={{ width: `${p}%` }} />
                    </div>
                    <span className="pct-text">better than {p}%</span>
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <p className="muted small-text">
        Your median over {view.games} recent games against {view.reference}. 50% means typical for that rank.
        Lane diffs aren't shown: against an equal opponent they're always near zero.
      </p>
    </>
  );
}
