import { useMemo } from "react";
import type { Asset } from "./data";
import { gap } from "./data";

// Stable per-symbol angle so a given asset always lands in the same spot
// across re-renders/refreshes (pure data would otherwise jitter blips
// around every 60s poll, which reads as noise rather than signal).
function hashAngle(symbol: string): number {
  let h = 7;
  for (let i = 0; i < symbol.length; i++) h = (h * 31 + symbol.charCodeAt(i)) >>> 0;
  return h % 360;
}

export function RadarScreen({
  assets,
  onSelect,
  emptyLabel,
}: {
  assets: Asset[];
  onSelect: (a: Asset) => void;
  emptyLabel: string;
}) {
  const size = 440;
  const center = size / 2;
  const maxRadius = center - 34;
  const rings = [0.25, 0.5, 0.75, 1];

  const blips = useMemo(
    () =>
      assets.map((a) => {
        const scoreFrac = Math.min(Math.max(a.score / 100, 0), 1);
        const radius = maxRadius * (0.08 + scoreFrac * 0.92);
        const angleRad = (hashAngle(a.symbol) * Math.PI) / 180;
        const x = center + radius * Math.cos(angleRad);
        const y = center + radius * Math.sin(angleRad);
        const r = 3.5 + scoreFrac * 5.5;
        return { a, x, y, r, positive: gap(a) >= 0 };
      }),
    [assets],
  );

  return (
    <div className="radar-screen">
      <div className="radar-sweep-overlay" />
      <svg viewBox={`0 0 ${size} ${size}`} className="radar-screen-svg">
        <circle cx={center} cy={center} r={maxRadius + 8} className="radar-screen-bg" />
        {rings.map((f) => (
          <circle
            key={f}
            cx={center}
            cy={center}
            r={maxRadius * f}
            className="radar-ring-line"
          />
        ))}
        <line
          x1={center}
          y1={center - maxRadius}
          x2={center}
          y2={center + maxRadius}
          className="radar-axis-line"
        />
        <line
          x1={center - maxRadius}
          y1={center}
          x2={center + maxRadius}
          y2={center}
          className="radar-axis-line"
        />
        {blips.map(({ a, x, y, r, positive }) => (
          <circle
            key={a.symbol}
            cx={x}
            cy={y}
            r={r}
            className={`radar-blip ${positive ? "positive" : "negative"}`}
            onClick={() => onSelect(a)}
          >
            <title>{`${a.symbol} · ${a.score}/100 · ${gap(a) >= 0 ? "+" : ""}${gap(a).toFixed(2)}%`}</title>
          </circle>
        ))}
      </svg>
      {assets.length === 0 && <p className="radar-screen-empty">{emptyLabel}</p>}
    </div>
  );
}
