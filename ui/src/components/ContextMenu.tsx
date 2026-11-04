import { useEffect, useRef } from "react";

export interface MenuItem {
  label: string;
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean;
  onClick: () => void;
}

interface Props {
  x: number;
  y: number;
  items: (MenuItem | "separator")[];
  onClose: () => void;
}

export function ContextMenu({ x, y, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  // Keep the menu inside the window.
  const left = Math.min(x, window.innerWidth - 240);
  const top = Math.min(y, window.innerHeight - items.length * 28 - 12);

  return (
    <div ref={ref} className="menu" style={{ left, top }} role="menu">
      {items.map((it, i) =>
        it === "separator" ? (
          <div key={i} className="menu-sep" />
        ) : (
          <button
            key={i}
            role="menuitem"
            className={`menu-item ${it.danger ? "danger" : ""}`}
            disabled={it.disabled}
            onClick={() => {
              onClose();
              it.onClick();
            }}
          >
            <span>{it.label}</span>
            {it.shortcut && <span className="kbd">{it.shortcut}</span>}
          </button>
        ),
      )}
    </div>
  );
}
