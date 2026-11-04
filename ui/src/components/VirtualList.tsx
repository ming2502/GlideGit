import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

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
}

/**
 * Fixed-row-height virtual list over a row count. Uses spacer elements (not
 * absolute rows) so the content can grow horizontally for long lines (diffs).
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
}: RowsProps) {
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

  const visStart = Math.min(count, Math.floor(scrollTop / rowHeight));
  const visEnd = Math.min(count, Math.ceil((scrollTop + height) / rowHeight));
  useEffect(() => {
    onRangeChange?.(visStart, visEnd);
  }, [visStart, visEnd, onRangeChange]);

  const start = Math.max(0, visStart - overscan);
  const end = Math.min(count, visEnd + overscan);

  const rows: ReactNode[] = [];
  for (let i = start; i < end; i++) rows.push(renderRow(i));

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
        <div style={{ height: (count - end) * rowHeight }} />
      </div>
    </div>
  );
}

interface ListProps<T> extends Omit<RowsProps, "count" | "renderRow"> {
  items: T[];
  render: (item: T, index: number) => ReactNode;
}

/** [`VirtualRows`] over an in-memory array. */
export function VirtualList<T>({ items, render, ...rest }: ListProps<T>) {
  return <VirtualRows count={items.length} renderRow={(i) => render(items[i], i)} {...rest} />;
}
