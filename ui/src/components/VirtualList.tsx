import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

/**
 * Tallest scroll area we create. Browsers cap element height (Chromium and
 * WebKit at about 33.5 million px, i.e. ~1.3 M rows of 26 px); longer lists
 * map their virtual height onto this many pixels of scrollbar.
 */
const MAX_SCROLL_PX = 8_000_000;

/** The part of the virtual content that is on screen, in virtual pixels. */
export interface Viewport {
  top: number;
  height: number;
}

interface RowsProps {
  /** Number of rows; rows need not be loaded to be counted. */
  count: number;
  rowHeight: number;
  renderRow: (index: number) => ReactNode;
  className?: string;
  overscan?: number;
  /** Keep this row visible (e.g. keyboard selection). */
  scrollToIndex?: number | null;
  onKeyDown?: (e: KeyboardEvent<HTMLDivElement>) => void;
  tabIndex?: number;
  /** Rows currently on screen, `[start, end)`, reported when they change. */
  onRangeChange?: (start: number, end: number) => void;
  /** Drawn over the visible area (e.g. a canvas); does not scroll or take clicks. */
  overlay?: (viewport: Viewport) => ReactNode;
}

/**
 * Fixed-row-height virtual list over a row count. Uses spacer elements (not
 * absolute rows) so the content can grow horizontally for long lines (diffs).
 *
 * The scroll position is kept in *virtual* pixels (row × height). For lists
 * taller than {@link MAX_SCROLL_PX} the scrollbar is scaled; wheel scrolling
 * is then applied in virtual pixels so it keeps its normal speed.
 */
export function VirtualRows({
  count,
  rowHeight,
  renderRow,
  className,
  overscan = 12,
  scrollToIndex,
  onKeyDown,
  tabIndex,
  onRangeChange,
  overlay,
}: RowsProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(600);
  // `physical` is the element's scrollTop; `top` the same position in virtual px.
  const [pos, setPos] = useState({ physical: 0, top: 0 });
  const lastSet = useRef<number | null>(null);

  const virtualH = count * rowHeight;
  const physicalH = Math.min(virtualH, MAX_SCROLL_PX);
  const scaled = virtualH > physicalH;
  /** Largest virtual top and virtual px per scrollbar px, for a viewport height. */
  const metrics = (h: number) => {
    const maxTop = Math.max(0, virtualH - h);
    return { maxTop, ratio: scaled ? maxTop / Math.max(1, physicalH - h) : 1 };
  };
  const { maxTop, ratio } = metrics(height);

  // Latest values for the event handlers below.
  const live = useRef({ metrics, scaled, top: pos.top });
  live.current = { metrics, scaled, top: pos.top };

  const scrollToVirtual = (top: number) => {
    const el = ref.current;
    if (!el) return;
    // The element's own height: the state may lag a resize by a frame.
    const { ratio, maxTop } = live.current.metrics(el.clientHeight);
    const t = Math.max(0, Math.min(maxTop, top));
    const physical = t / ratio;
    lastSet.current = physical;
    el.scrollTop = physical;
    setPos({ physical, top: t });
  };

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setHeight(el.clientHeight);
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Wheel in virtual pixels when the scrollbar is scaled.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!live.current.scaled || e.deltaY === 0) return;
      e.preventDefault();
      const unit = e.deltaMode === 1 ? rowHeight : e.deltaMode === 2 ? el.clientHeight : 1;
      scrollToVirtual(live.current.top + e.deltaY * unit);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [rowHeight]);

  // Keep the same rows on screen when the list or the viewport changes size
  // (a scaled scrollbar maps virtual positions differently then).
  useEffect(() => {
    if (pos.top > maxTop || scaled) scrollToVirtual(pos.top);
  }, [maxTop, height]);

  useEffect(() => {
    if (scrollToIndex == null || scrollToIndex < 0) return;
    const top = scrollToIndex * rowHeight;
    const { top: cur } = live.current;
    const h = ref.current?.clientHeight ?? height;
    if (top < cur) scrollToVirtual(top);
    else if (top + rowHeight > cur + h) scrollToVirtual(top + rowHeight - h);
  }, [scrollToIndex, rowHeight]);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const physical = e.currentTarget.scrollTop;
    // Our own scrollTop assignment (possibly rounded by the browser).
    if (lastSet.current != null && Math.abs(physical - lastSet.current) < 1) return;
    lastSet.current = null;
    setPos({ physical, top: Math.min(maxTop, physical * ratio) });
  };

  const visStart = Math.min(count, Math.floor(pos.top / rowHeight));
  const visEnd = Math.min(count, Math.ceil((pos.top + height) / rowHeight));
  useEffect(() => {
    onRangeChange?.(visStart, visEnd);
  }, [visStart, visEnd, onRangeChange]);

  const start = Math.max(0, visStart - overscan);
  const end = Math.min(count, visEnd + overscan);
  // Place row `start` so that the virtual position `pos.top` sits at the top edge.
  const blockTop = Math.max(0, pos.physical - (pos.top - start * rowHeight));
  const bottom = Math.max(0, physicalH - blockTop - (end - start) * rowHeight);

  const rows: ReactNode[] = [];
  for (let i = start; i < end; i++) rows.push(renderRow(i));

  return (
    <div ref={ref} className={`vlist ${className ?? ""}`} onScroll={onScroll} onKeyDown={onKeyDown} tabIndex={tabIndex}>
      {overlay && (
        <div className="vlist-overlay" style={{ height, marginBottom: -height }}>
          {overlay({ top: pos.top, height })}
        </div>
      )}
      <div className="vlist-inner">
        <div style={{ height: blockTop }} />
        {rows}
        <div style={{ height: bottom }} />
      </div>
    </div>
  );
}

interface ListProps<T> extends Omit<RowsProps, "count" | "renderRow"> {
  items: T[];
  render: (item: T, index: number) => ReactNode;
}

/** {@link VirtualRows} over an in-memory array. */
export function VirtualList<T>({ items, render, ...rest }: ListProps<T>) {
  return <VirtualRows count={items.length} renderRow={(i) => render(items[i], i)} {...rest} />;
}
