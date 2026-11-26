// Layout helpers of our own: a resizable splitter (mouse and keyboard) and a virtualised list.
import type { KeyboardEvent, PointerEvent, ReactNode } from 'react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Dialog as RDialog } from 'radix-ui';

/**
 * A separator that resizes the panel next to it. Dragging is optional: arrow keys move it by
 * 16 px (Shift: 64 px), Home and End go to the limits (WCAG 2.2: dragging has an alternative).
 */
export function Splitter({
  orientation,
  value,
  min,
  max,
  onChange,
  label,
  invert = false,
}: {
  /** "vertical": a vertical bar between columns; "horizontal": a bar between rows. */
  orientation: 'vertical' | 'horizontal';
  value: number;
  min: number;
  max: number;
  onChange: (v: number) => void;
  label: string;
  /** The panel grows when the bar moves left or up (an inspector on the right, a bottom panel). */
  invert?: boolean;
}) {
  const start = useRef<{ pos: number; value: number } | null>(null);
  const clamp = (v: number) => Math.min(max, Math.max(min, Math.round(v)));
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    start.current = { pos: orientation === 'vertical' ? e.clientX : e.clientY, value };
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!start.current) return;
    const delta = (orientation === 'vertical' ? e.clientX : e.clientY) - start.current.pos;
    onChange(clamp(start.current.value + (invert ? -delta : delta)));
  };
  const onPointerUp = () => {
    start.current = null;
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 64 : 16;
    const grow = orientation === 'vertical' ? (invert ? 'ArrowLeft' : 'ArrowRight') : invert ? 'ArrowUp' : 'ArrowDown';
    const shrink = orientation === 'vertical' ? (invert ? 'ArrowRight' : 'ArrowLeft') : invert ? 'ArrowDown' : 'ArrowUp';
    const next = e.key === grow ? value + step : e.key === shrink ? value - step : e.key === 'Home' ? min : e.key === 'End' ? max : null;
    if (next !== null) {
      e.preventDefault();
      onChange(clamp(next));
    }
  };
  return (
    <div
      role="separator"
      aria-orientation={orientation}
      aria-label={label}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      className={`lg-splitter lg-splitter--${orientation}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onKeyDown={onKeyDown}
    />
  );
}

/** Renders only the rows in view (doc 08: decoded tables of millions of rows). */
export function VirtualList({
  count,
  rowHeight,
  render,
  label,
  overscan = 8,
  scrollTo,
}: {
  count: number;
  rowHeight: number;
  render: (index: number) => ReactNode;
  label: string;
  overscan?: number;
  /** Scrolls this row into view when it changes. */
  scrollTo?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(400);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => {
    const el = ref.current;
    if (!el || scrollTo === undefined) return;
    const y = scrollTo * rowHeight;
    if (y < el.scrollTop || y + rowHeight > el.scrollTop + el.clientHeight) {
      el.scrollTop = y - el.clientHeight / 2;
    }
  }, [scrollTo, rowHeight]);
  const first = Math.max(0, Math.floor(top / rowHeight) - overscan);
  const last = Math.min(count, Math.ceil((top + height) / rowHeight) + overscan);
  const rows: ReactNode[] = [];
  for (let i = first; i < last; i++) {
    rows.push(
      <div key={i} role="listitem" className="lg-vlist__row" style={{ top: i * rowHeight, height: rowHeight }}>
        {render(i)}
      </div>,
    );
  }
  return (
    <div ref={ref} className="lg-vlist" role="list" aria-label={label} onScroll={(e) => setTop(e.currentTarget.scrollTop)} tabIndex={0}>
      <div style={{ height: count * rowHeight, position: 'relative' }}>{rows}</div>
    </div>
  );
}

export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  wide,
  header,
  className,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children?: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
  /** Shown under the title and stays in sight while the body scrolls (a step bar, say). */
  header?: ReactNode;
  className?: string;
}) {
  return (
    <RDialog.Root open={open} onOpenChange={onOpenChange}>
      <RDialog.Portal>
        <RDialog.Overlay className="lg-dialog__scrim" />
        <RDialog.Content className={['lg-dialog', wide && 'lg-dialog--wide', className].filter(Boolean).join(' ')}>
          {/* the title and the buttons stay in sight; a long form scrolls between them */}
          <div className="lg-dialog__header">
            <RDialog.Title className="lg-dialog__title">{title}</RDialog.Title>
            {description ? (
              <RDialog.Description className="lg-dialog__description">{description}</RDialog.Description>
            ) : (
              <RDialog.Description className="lg-visually-hidden">{title}</RDialog.Description>
            )}
            {header}
          </div>
          {children !== undefined && children !== null && children !== false && <div className="lg-dialog__body">{children}</div>}
          {footer && <div className="lg-dialog__footer">{footer}</div>}
        </RDialog.Content>
      </RDialog.Portal>
    </RDialog.Root>
  );
}

/** Keeps a value in localStorage (panel sizes and the like, doc 08: panels remember their size). */
export function useStored<T>(key: string, initial: T): [T, (v: T) => void] {
  const [v, setV] = useState<T>(() => {
    try {
      const s = localStorage.getItem(key);
      return s === null ? initial : (JSON.parse(s) as T);
    } catch {
      return initial;
    }
  });
  const set = useCallback(
    (next: T) => {
      setV(next);
      try {
        localStorage.setItem(key, JSON.stringify(next));
      } catch {
        // storage can be full or off; the value still holds for this session
      }
    },
    [key],
  );
  return [v, set];
}
