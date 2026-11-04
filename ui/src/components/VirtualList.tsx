import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

interface Props<T> {
  items: T[];
  rowHeight: number;
  render: (item: T, index: number) => ReactNode;
  className?: string;
  overscan?: number;
  /** Keep this row visible (e.g. keyboard selection). */
  scrollToIndex?: number | null;
  onKeyDown?: (e: KeyboardEvent<HTMLDivElement>) => void;
  tabIndex?: number;
}

/**
 * Fixed-row-height virtual list. Uses spacer elements (not absolute rows) so
 * the content can grow horizontally for long lines (diffs).
 */
export function VirtualList<T>({
  items,
  rowHeight,
  render,
  className,
  overscan = 12,
  scrollToIndex,
  onKeyDown,
  tabIndex,
}: Props<T>) {
  const ref = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(600);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setHeight(el.clientHeight);
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el || scrollToIndex == null || scrollToIndex < 0) return;
    const top = scrollToIndex * rowHeight;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + rowHeight > el.scrollTop + el.clientHeight)
      el.scrollTop = top + rowHeight - el.clientHeight;
  }, [scrollToIndex, rowHeight]);

  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const end = Math.min(items.length, Math.ceil((scrollTop + height) / rowHeight) + overscan);

  const rows: ReactNode[] = [];
  for (let i = start; i < end; i++) rows.push(render(items[i], i));

  return (
    <div
      ref={ref}
      className={`vlist ${className ?? ""}`}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      onKeyDown={onKeyDown}
      tabIndex={tabIndex}
    >
      <div className="vlist-inner">
        <div style={{ height: start * rowHeight }} />
        {rows}
        <div style={{ height: (items.length - end) * rowHeight }} />
      </div>
    </div>
  );
}
