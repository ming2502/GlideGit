// Commit graph for the Log: one canvas over the visible rows, redrawn on
// scroll. Layout (columns and lines) comes precomputed from the backend; see
// crates/log-index/src/graph.rs for what the numbers mean.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { GraphRow } from "../bindings/GraphRow";
import type { Viewport } from "../components/VirtualList";

/** Width of one graph column, px. */
export const LANE_W = 14;
/** Columns drawn at most; wider graphs are clipped on the right. */
export const MAX_LANES = 24;
const PAD = 4;
const PALETTE_SIZE = 8;
const NODE_R = 3.5;

/** Columns used by a row (highest column mentioned + 1). */
export function graphWidth(g: GraphRow): number {
  let w = g.lane;
  for (const c of g.up) if (c > w) w = c;
  for (const c of g.down) if (c > w) w = c;
  return w + 1;
}

/** Pixel width of the graph column for `lanes` columns. */
export function graphPx(lanes: number): number {
  return Math.min(lanes, MAX_LANES) * LANE_W + PAD * 2;
}

/** Bumps when the colour scheme changes, so the canvas re-reads its palette. */
function useThemeVersion(): number {
  const [v, setV] = useState(0);
  useEffect(() => {
    const bump = () => setV((x) => x + 1);
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", bump);
    const mo = new MutationObserver(bump);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      mq.removeEventListener("change", bump);
      mo.disconnect();
    };
  }, []);
  return v;
}

interface Props {
  viewport: Viewport;
  rowHeight: number;
  /** Graph columns to draw (the graph column's width). */
  lanes: number;
  graph: (row: number) => GraphRow | undefined;
  /** Rows whose node is drawn hollow (merge commits). */
  isMerge: (row: number) => boolean;
  /** Changes whenever loaded rows change, to trigger a redraw. */
  dataVersion: number;
  /** Horizontal offset of the graph column inside a row, px. */
  left: number;
}

export function LogGraph({ viewport, rowHeight, lanes, graph, isMerge, dataVersion, left }: Props) {
  const ref = useRef<HTMLCanvasElement>(null);
  const theme = useThemeVersion();
  const width = graphPx(lanes);

  useLayoutEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const h = viewport.height;
    if (canvas.width !== Math.round(width * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(h * dpr);
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, h);

    const css = getComputedStyle(canvas);
    const palette = Array.from({ length: PALETTE_SIZE }, (_, k) => css.getPropertyValue(`--graph-${k}`).trim() || "#888");
    const bg = css.getPropertyValue("--bg").trim() || "#fff";
    const color = (col: number) => palette[col % PALETTE_SIZE];
    const x = (col: number) => PAD + col * LANE_W + LANE_W / 2;
    const half = rowHeight / 2;

    ctx.lineWidth = 1.6;
    ctx.lineCap = "round";
    const line = (a: number, y1: number, b: number, y2: number, c: string) => {
      if (a >= MAX_LANES && b >= MAX_LANES) return;
      ctx.strokeStyle = c;
      ctx.beginPath();
      ctx.moveTo(x(a), y1);
      if (a === b) ctx.lineTo(x(b), y2);
      else {
        const ym = (y1 + y2) / 2;
        ctx.bezierCurveTo(x(a), ym, x(b), ym, x(b), y2);
      }
      ctx.stroke();
    };

    // One row of margin each side so lines run into the edges.
    const first = Math.max(0, Math.floor(viewport.top / rowHeight) - 1);
    const last = Math.ceil((viewport.top + h) / rowHeight) + 1;
    for (let r = first; r < last; r++) {
      const g = graph(r);
      if (!g) continue;
      const y = r * rowHeight - viewport.top;
      // Upper half: the line's colour is that of the column it comes from;
      // lower half: of the column it goes to.
      for (let i = 0; i + 1 < g.up.length; i += 2) line(g.up[i], y, g.up[i + 1], y + half, color(g.up[i]));
      for (let i = 0; i + 1 < g.down.length; i += 2)
        line(g.down[i], y + half, g.down[i + 1], y + rowHeight, color(g.down[i + 1]));
    }
    for (let r = first; r < last; r++) {
      const g = graph(r);
      if (!g || g.lane >= MAX_LANES) continue;
      const cy = r * rowHeight - viewport.top + half;
      const c = color(g.lane);
      ctx.beginPath();
      ctx.arc(x(g.lane), cy, NODE_R, 0, Math.PI * 2);
      ctx.fillStyle = isMerge(r) ? bg : c;
      ctx.fill();
      ctx.lineWidth = 1.6;
      ctx.strokeStyle = c;
      ctx.stroke();
    }
  }, [viewport.top, viewport.height, width, rowHeight, graph, isMerge, dataVersion, theme]);

  return (
    <canvas
      ref={ref}
      className="log-graph"
      style={{ width, height: viewport.height, left }}
      aria-hidden="true"
    />
  );
}
