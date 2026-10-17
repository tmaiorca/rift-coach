// Minimal SVG line chart with a hover crosshair. No chart library: the app
// needs a handful of time series, and this keeps the bundle small.
import { useRef, useState } from "react";

export interface ChartSeries {
  label: string;
  color: string;
  values: number[];
  dashed?: boolean;
}

interface Props {
  x: number[];
  series: ChartSeries[];
  height?: number;
  /** Shade above/below zero (for diff charts). */
  zeroBand?: boolean;
  formatY?: (v: number) => string;
  formatX?: (v: number) => string;
  /** Vertical markers, e.g. deaths: [{x, label, color}] */
  markers?: { x: number; label: string; color: string }[];
}

const PAD = { l: 48, r: 12, t: 10, b: 22 };

export function LineChart({ x, series, height = 200, zeroBand, formatY, formatX, markers = [] }: Props) {
  const ref = useRef<SVGSVGElement>(null);
  const [hover, setHover] = useState<number | null>(null);
  const width = 640;
  const fy = formatY ?? ((v: number) => Math.round(v).toLocaleString());
  const fx = formatX ?? ((v: number) => `${Math.round(v)}m`);

  if (x.length < 2 || series.length === 0) return <p className="muted">No timeline data.</p>;
  const all = series.flatMap((s) => s.values);
  let lo = Math.min(...all, zeroBand ? 0 : Infinity);
  let hi = Math.max(...all, zeroBand ? 0 : -Infinity);
  if (zeroBand) {
    const m = Math.max(Math.abs(lo), Math.abs(hi), 1);
    lo = -m;
    hi = m;
  }
  if (hi === lo) hi = lo + 1;
  const x0 = x[0] ?? 0;
  const x1 = x[x.length - 1] ?? 0;
  const at = (i: number) => x[i] ?? 0;
  const sx = (v: number) => PAD.l + ((v - x0) / (x1 - x0 || 1)) * (width - PAD.l - PAD.r);
  const sy = (v: number) => PAD.t + (1 - (v - lo) / (hi - lo)) * (height - PAD.t - PAD.b);
  const path = (vals: number[]) =>
    vals.map((v, i) => `${i ? "L" : "M"}${sx(at(i)).toFixed(1)},${sy(v).toFixed(1)}`).join("");
  const yTicks = [lo, lo + (hi - lo) / 2, hi];
  const xTicks = x
    .filter((v, i) => i === 0 || Math.round(v) % 5 === 0)
    .filter((v, i, a) => a.indexOf(v) === i);

  const onMove = (e: React.MouseEvent) => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return;
    const px = ((e.clientX - r.left) / r.width) * width;
    let best = 0;
    for (let i = 1; i < x.length; i++) if (Math.abs(sx(at(i)) - px) < Math.abs(sx(at(best)) - px)) best = i;
    setHover(best);
  };

  return (
    <div className="chart">
      <svg
        ref={ref}
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        onMouseMove={onMove}
        onMouseLeave={() => setHover(null)}
      >
        {zeroBand && (
          <>
            <rect
              x={PAD.l}
              y={PAD.t}
              width={width - PAD.l - PAD.r}
              height={sy(0) - PAD.t}
              className="band-up"
            />
            <rect
              x={PAD.l}
              y={sy(0)}
              width={width - PAD.l - PAD.r}
              height={height - PAD.b - sy(0)}
              className="band-down"
            />
          </>
        )}
        {yTicks.map((v) => (
          <g key={v}>
            <line x1={PAD.l} x2={width - PAD.r} y1={sy(v)} y2={sy(v)} className="grid" />
            <text x={PAD.l - 6} y={sy(v) + 4} className="axis" textAnchor="end">
              {fy(v)}
            </text>
          </g>
        ))}
        {zeroBand && <line x1={PAD.l} x2={width - PAD.r} y1={sy(0)} y2={sy(0)} className="zero" />}
        {xTicks.map((v) => (
          <text key={v} x={sx(v)} y={height - 6} className="axis" textAnchor="middle">
            {fx(v)}
          </text>
        ))}
        {markers.map((m, i) => (
          <line
            key={i}
            x1={sx(m.x)}
            x2={sx(m.x)}
            y1={PAD.t}
            y2={height - PAD.b}
            stroke={m.color}
            className="marker"
          >
            <title>{m.label}</title>
          </line>
        ))}
        {series.map((s) => (
          <path
            key={s.label}
            d={path(s.values)}
            fill="none"
            stroke={s.color}
            strokeWidth={2}
            strokeDasharray={s.dashed ? "5 4" : undefined}
            vectorEffect="non-scaling-stroke"
          />
        ))}
        {hover !== null && (
          <line x1={sx(at(hover))} x2={sx(at(hover))} y1={PAD.t} y2={height - PAD.b} className="crosshair" />
        )}
      </svg>
      <div className="chart-legend">
        {series.map((s) => (
          <span key={s.label}>
            <i style={{ background: s.color }} /> {s.label}
            {hover !== null && <b> {fy(s.values[hover] ?? 0)}</b>}
          </span>
        ))}
        {hover !== null && <span className="muted">at {fx(at(hover))}</span>}
      </div>
    </div>
  );
}
