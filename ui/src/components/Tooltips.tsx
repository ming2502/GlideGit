// In-app tooltips instead of the system ones. Components keep writing a plain
// `title`; this layer moves every `title` in the document to `data-tip` as
// soon as it appears or changes (so the system tooltip never shows) and draws
// a styled bubble after a short hover.
import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** Hover this long before a tip shows. */
const SHOW_DELAY = 500;
/** Moving on to another element within this time after a tip shows its tip at once. */
const SWITCH_WINDOW = 400;
const GAP = 6;
const MARGIN = 8;

/** Move `title` to `data-tip`. Elements without text get it as their accessible name too. */
export function adoptTitle(el: Element): void {
  const title = el.getAttribute("title");
  if (title === null) return;
  el.removeAttribute("title");
  if (title) el.setAttribute("data-tip", title);
  else el.removeAttribute("data-tip");
  if (el.hasAttribute("data-tip-label") || (!el.hasAttribute("aria-label") && !el.textContent?.trim())) {
    el.setAttribute("data-tip-label", "");
    if (title) el.setAttribute("aria-label", title);
    else el.removeAttribute("aria-label");
  }
}

/** Adopt the titles of `node` and everything inside it. */
export function adoptTitles(node: Node): void {
  if (!(node instanceof Element)) return;
  adoptTitle(node);
  node.querySelectorAll("[title]").forEach(adoptTitle);
}

type Tip = { text: string; anchor: DOMRect; x: number };

export function Tooltips() {
  const [tip, setTip] = useState<Tip | null>(null);
  const bubble = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useEffect(() => {
    let anchor: Element | null = null;
    /** The element whose tip was dismissed by a click: not shown again until the pointer leaves it. */
    let dismissed: Element | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let visible = false;
    /** When the last tip went away. */
    let hiddenAt = 0;
    let lastX = 0;

    const show = () => {
      const text = anchor?.getAttribute("data-tip");
      if (!anchor?.isConnected || !text) return;
      setTip({ text, anchor: anchor.getBoundingClientRect(), x: lastX });
      visible = true;
    };
    const hide = () => {
      clearTimeout(timer);
      if (visible) hiddenAt = Date.now();
      visible = false;
      setTip(null);
    };

    const over = (e: MouseEvent) => {
      lastX = e.clientX;
      const target = e.target instanceof Element ? e.target : null;
      // The nearest `title` wins for the system tooltip, so clear the whole chain.
      for (let el = target; el; el = el.parentElement) if (el.hasAttribute("title")) adoptTitle(el);
      const next = target?.closest("[data-tip]") ?? null;
      if (next === anchor) return;
      hide();
      anchor = next;
      if (next !== dismissed) dismissed = null;
      if (!next || dismissed) return;
      if (Date.now() - hiddenAt < SWITCH_WINDOW) show();
      else timer = setTimeout(show, SHOW_DELAY);
    };
    const out = (e: MouseEvent) => {
      const to = e.relatedTarget instanceof Node ? e.relatedTarget : null;
      if (anchor && to && anchor.contains(to)) return;
      hide();
      anchor = null;
    };
    const move = (e: MouseEvent) => {
      lastX = e.clientX;
    };
    const dismiss = () => {
      if (anchor) dismissed = anchor;
      hide();
      anchor = null;
    };

    adoptTitles(document.body);
    const observer = new MutationObserver((records) => {
      for (const r of records) {
        if (r.type === "attributes") {
          const el = r.target as Element;
          if (!el.hasAttribute("title")) continue;
          adoptTitle(el);
          // The hovered element's tip changed (e.g. "stage" → "staged").
          if (el === anchor && visible) {
            const text = el.getAttribute("data-tip");
            if (text) setTip((cur) => cur && { ...cur, text });
            else hide();
          }
        } else r.addedNodes.forEach(adoptTitles);
      }
    });
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["title"] });

    document.addEventListener("mouseover", over, true);
    document.addEventListener("mouseout", out, true);
    document.addEventListener("mousemove", move, { capture: true, passive: true });
    document.addEventListener("mousedown", dismiss, true);
    document.addEventListener("keydown", dismiss, true);
    document.addEventListener("wheel", dismiss, { capture: true, passive: true });
    window.addEventListener("blur", dismiss);
    window.addEventListener("scroll", dismiss, true);
    return () => {
      clearTimeout(timer);
      observer.disconnect();
      document.removeEventListener("mouseover", over, true);
      document.removeEventListener("mouseout", out, true);
      document.removeEventListener("mousemove", move, true);
      document.removeEventListener("mousedown", dismiss, true);
      document.removeEventListener("keydown", dismiss, true);
      document.removeEventListener("wheel", dismiss, true);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("scroll", dismiss, true);
    };
  }, []);

  // Below the element (above when there is no room), kept inside the window.
  // Narrow elements center the tip; wide ones (rows) put it at the pointer.
  useLayoutEffect(() => {
    const el = bubble.current;
    if (!tip || !el) return setPos(null);
    const { width, height } = el.getBoundingClientRect();
    const a = tip.anchor;
    const cx = a.width <= 240 ? a.left + a.width / 2 : tip.x;
    const left = Math.max(MARGIN, Math.min(cx - width / 2, window.innerWidth - width - MARGIN));
    const below = a.bottom + GAP;
    const top = below + height + MARGIN <= window.innerHeight ? below : Math.max(MARGIN, a.top - GAP - height);
    setPos({ left, top });
  }, [tip]);

  if (!tip) return null;
  return (
    <div
      ref={bubble}
      className="tooltip"
      role="tooltip"
      style={pos ? { left: pos.left, top: pos.top } : { left: 0, top: 0, visibility: "hidden" }}
    >
      {tip.text}
    </div>
  );
}
