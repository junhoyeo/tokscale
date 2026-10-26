"use client";

/**
 * InstantTooltip: a hover and focus tooltip with no delay.
 *
 * Native title="" tooltips wait about a second and never show on keyboard
 * focus. This one opens as soon as the pointer is over the trigger or the
 * trigger gets keyboard focus, and closes on pointer leave, blur and Escape.
 * The pointer can move onto the tooltip without closing it. It renders in a
 * portal with fixed positioning (like the Select listbox), so overflow,
 * table scrollers and stacking contexts cannot clip it. It sits above the
 * trigger, or below it when there is no room above, and stays inside the
 * viewport.
 *
 * The tooltip is the trigger element itself (`as`, default span), not a
 * wrapper, so it keeps the layout and the focus target of what it replaces.
 *
 * Row links: a row marked data-row-link is covered by its primary link's
 * ::after, so a trigger inside it never gets the pointer. The tooltip then
 * follows the pointer over the row and opens while the pointer is over the
 * trigger's box: a value still explains itself, and a click on it still
 * opens the row.
 *
 * Screen readers: with `describe` (default) the trigger points at the open
 * tooltip with aria-describedby. Set `describe={false}` when the trigger
 * already carries the same text (visually hidden text or an aria-label); the
 * tooltip is then hidden from assistive technology, so nothing is read twice.
 */
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ElementType,
  type FocusEvent,
  type MutableRefObject,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type Ref,
} from "react";
import { createPortal } from "react-dom";
import styled from "styled-components";

const GAP = 6;
const VIEWPORT_MARGIN = 8;

export interface InstantTooltipProps {
  /** Tooltip text. Nothing opens when it is empty. */
  content: ReactNode;
  /** Element or component to render as the trigger. Default "span". */
  as?: ElementType;
  /** Point the trigger at the open tooltip with aria-describedby. Default true. */
  describe?: boolean;
  /** Put a non-interactive trigger in the tab order, so keyboard users can open it. */
  focusable?: boolean;
  /** Also open while the link or button around the trigger has keyboard focus, for a badge inside a link. */
  openWithParentFocus?: boolean;
  className?: string;
  style?: CSSProperties;
  children?: ReactNode;
  /** Any other prop goes to the trigger. */
  [prop: string]: unknown;
}

type Handler<E> = ((event: E) => void) | undefined;

function inside(element: Element, x: number, y: number): boolean {
  for (const rect of Array.from(element.getClientRects())) {
    if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) return true;
  }
  return false;
}

/** A row cover is transparent to tooltips; an opaque sticky cell or another control is not. */
function visibleInRow(trigger: Element, row: Element, x: number, y: number): boolean {
  if (!inside(trigger, x, y)) return false;
  const hit = document.elementFromPoint(x, y);
  if (!hit) return false;
  if (trigger.contains(hit)) return true;
  const cover = hit.closest("[data-row-primary], [data-row-cover]");
  if (!cover || cover.closest("[data-row-link]") !== row) return false;
  const cell = cover.closest("td, th");
  // A primary link's ::after is confined to its sticky cell, unlike a whole-row cover.
  return !cell || cell === trigger.closest("td, th") || getComputedStyle(cell).position !== "sticky";
}

function assignRef<T>(ref: Ref<T> | undefined, value: T): void {
  if (typeof ref === "function") ref(value);
  else if (ref) (ref as MutableRefObject<T>).current = value;
}

function keyboardFocus(element: Element): boolean {
  try {
    return element.matches(":focus-visible");
  } catch {
    return true;
  }
}

/** Fixed position above the trigger, below when there is no room, clamped to the viewport. */
function place(trigger: Element, tip: HTMLElement): void {
  const rect = trigger.getBoundingClientRect();
  const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
  const viewportHeight = document.documentElement.clientHeight || window.innerHeight;
  const width = tip.offsetWidth;
  const height = tip.offsetHeight;
  const above = rect.top - GAP - height;
  const below = rect.bottom + GAP;
  const fitsBelow = below + height <= viewportHeight - VIEWPORT_MARGIN;
  const placeBelow = above < VIEWPORT_MARGIN && fitsBelow;
  const top = placeBelow ? below : Math.max(VIEWPORT_MARGIN, above);
  const centered = rect.left + rect.width / 2 - width / 2;
  const left = Math.max(VIEWPORT_MARGIN, Math.min(centered, viewportWidth - VIEWPORT_MARGIN - width));
  tip.style.top = `${Math.round(top)}px`;
  tip.style.left = `${Math.round(left)}px`;
  tip.dataset.placement = placeBelow ? "bottom" : "top";
}

export function InstantTooltip({
  content,
  as,
  describe = true,
  focusable = false,
  openWithParentFocus = false,
  children,
  ...rest
}: InstantTooltipProps) {
  const Trigger = (as ?? "span") as ElementType;
  const triggerRef = useRef<Element | null>(null);
  const tipRef = useRef<HTMLDivElement | null>(null);
  const rowPointer = useRef<{ x: number; y: number } | null>(null);
  const flags = useRef({ hover: false, focus: false, dismissed: false });
  const [open, setOpen] = useState(false);
  const [portalTarget, setPortalTarget] = useState<HTMLElement | null>(null);
  // Ids from useId appear only while the tooltip is open, so they never reach server HTML.
  const tipId = `${useId()}tooltip`;
  const enabled = content !== null && content !== undefined && content !== false && content !== "";

  // The trigger needs its own ref; keep a ref the caller passed (React 19 passes ref as a prop).
  const callerRef = rest.ref as Ref<Element> | undefined;
  const setTriggerRef = useCallback(
    (node: Element | null) => {
      triggerRef.current = node;
      assignRef(callerRef, node);
    },
    [callerRef],
  );

  const sync = useCallback(() => {
    const { hover, focus, dismissed } = flags.current;
    const next = (hover || focus) && !dismissed;
    if (next) {
      // Inside a modal <dialog> (the top layer) the tooltip must render in the dialog to show above it.
      const target = (triggerRef.current?.closest("dialog") as HTMLElement | null) ?? document.body;
      setPortalTarget((current) => current ?? target);
    }
    setOpen(next);
  }, []);

  const setHover = useCallback(
    (hover: boolean) => {
      if (flags.current.hover === hover) return;
      flags.current.hover = hover;
      if (!hover && !flags.current.focus) flags.current.dismissed = false;
      sync();
    },
    [sync],
  );

  // Inside a row link the stretched link covers the trigger: follow the pointer over the row instead.
  useEffect(() => {
    const trigger = triggerRef.current;
    const row = trigger?.closest("[data-row-link]");
    if (!trigger || !row || !enabled) return;
    const onMove = (event: PointerEvent) => {
      if (event.pointerType === "touch") return;
      rowPointer.current = { x: event.clientX, y: event.clientY };
      setHover(visibleInRow(trigger, row, event.clientX, event.clientY));
    };
    const onLeave = (event: PointerEvent) => {
      rowPointer.current = null;
      const next = event.relatedTarget;
      if (next instanceof Node && tipRef.current?.contains(next)) return;
      setHover(false);
    };
    const onDown = (event: PointerEvent) => {
      if (!visibleInRow(trigger, row, event.clientX, event.clientY)) return;
      flags.current.dismissed = true;
      sync();
    };
    row.addEventListener("pointermove", onMove as EventListener);
    row.addEventListener("pointerleave", onLeave as EventListener);
    row.addEventListener("pointerdown", onDown as EventListener);
    return () => {
      row.removeEventListener("pointermove", onMove as EventListener);
      row.removeEventListener("pointerleave", onLeave as EventListener);
      row.removeEventListener("pointerdown", onDown as EventListener);
    };
  }, [enabled, setHover, sync]);

  // A badge inside a link opens with the link's keyboard focus.
  useEffect(() => {
    const host = openWithParentFocus ? triggerRef.current?.parentElement?.closest("a[href], button") : null;
    if (!host || !enabled) return;
    const onFocus = () => {
      flags.current.focus = keyboardFocus(host);
      sync();
    };
    const onBlur = () => {
      flags.current.focus = false;
      flags.current.dismissed = false;
      sync();
    };
    host.addEventListener("focus", onFocus);
    host.addEventListener("blur", onBlur);
    return () => {
      host.removeEventListener("focus", onFocus);
      host.removeEventListener("blur", onBlur);
    };
  }, [openWithParentFocus, enabled, sync]);

  // Escape closes the tooltip until the pointer leaves or focus moves.
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      flags.current.dismissed = true;
      sync();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, sync]);

  // Place before paint, and follow the trigger on scroll and resize.
  useLayoutEffect(() => {
    if (!open) return;
    const trigger = triggerRef.current;
    const tip = tipRef.current;
    if (!trigger || !tip) return;
    let frame = 0;
    const update = () => {
      frame = 0;
      if (!trigger.isConnected) {
        flags.current = { hover: false, focus: false, dismissed: false };
        setOpen(false);
        return;
      }
      place(trigger, tip);
      const point = rowPointer.current;
      const row = trigger.closest("[data-row-link]");
      if (point && row && !visibleInRow(trigger, row, point.x, point.y)) setHover(false);
    };
    const schedule = () => {
      if (frame === 0) frame = window.requestAnimationFrame(update);
    };
    update();
    window.addEventListener("scroll", schedule, true);
    window.addEventListener("resize", schedule);
    return () => {
      if (frame !== 0) window.cancelAnimationFrame(frame);
      window.removeEventListener("scroll", schedule, true);
      window.removeEventListener("resize", schedule);
    };
  }, [open, content, setHover]);

  const inRow = () => Boolean(triggerRef.current?.closest("[data-row-link]"));
  const theirs = <E,>(name: string) => rest[name] as Handler<E>;

  // Inside a row link the row's listeners above own the pointer; these handle every other trigger.
  const onPointerEnter = (event: ReactPointerEvent<Element>) => {
    theirs<ReactPointerEvent<Element>>("onPointerEnter")?.(event);
    if (event.pointerType !== "touch" && !inRow()) setHover(true);
  };
  const onPointerLeave = (event: ReactPointerEvent<Element>) => {
    theirs<ReactPointerEvent<Element>>("onPointerLeave")?.(event);
    if (inRow()) return;
    const next = event.relatedTarget;
    if (next instanceof Node && tipRef.current?.contains(next)) return;
    setHover(false);
  };
  // A press (a click on a link or button) closes the tooltip until the pointer leaves.
  const onPointerDown = (event: ReactPointerEvent<Element>) => {
    theirs<ReactPointerEvent<Element>>("onPointerDown")?.(event);
    if (inRow()) return;
    flags.current.dismissed = true;
    sync();
  };
  const onFocus = (event: FocusEvent<Element>) => {
    theirs<FocusEvent<Element>>("onFocus")?.(event);
    flags.current.focus = keyboardFocus(event.target);
    sync();
  };
  const onBlur = (event: FocusEvent<Element>) => {
    theirs<FocusEvent<Element>>("onBlur")?.(event);
    flags.current.focus = false;
    flags.current.dismissed = false;
    sync();
  };

  const triggerProps: Record<string, unknown> = { ...rest };
  if (enabled) {
    if (focusable && rest.tabIndex === undefined) triggerProps.tabIndex = 0;
    if (describe && open) {
      const existing = typeof rest["aria-describedby"] === "string" ? rest["aria-describedby"] : "";
      triggerProps["aria-describedby"] = existing ? `${existing} ${tipId}` : tipId;
    }
  }

  delete triggerProps.ref;

  const tooltip =
    enabled && open && portalTarget ? (
      <Tip
        ref={tipRef}
        id={tipId}
        role={describe ? "tooltip" : undefined}
        aria-hidden={describe ? undefined : true}
        onPointerLeave={(event) => {
          const trigger = triggerRef.current;
          const next = event.relatedTarget;
          if (trigger && next instanceof Node && trigger.contains(next)) return;
          const row = trigger?.closest("[data-row-link]");
          if (trigger && (row
            ? visibleInRow(trigger, row, event.clientX, event.clientY)
            : inside(trigger, event.clientX, event.clientY))) return;
          setHover(false);
        }}
      >
        {content}
      </Tip>
    ) : null;

  return (
    <>
      <Trigger
        {...triggerProps}
        ref={setTriggerRef}
        onPointerEnter={enabled ? onPointerEnter : rest.onPointerEnter}
        onPointerLeave={enabled ? onPointerLeave : rest.onPointerLeave}
        onPointerDown={enabled ? onPointerDown : rest.onPointerDown}
        onFocus={enabled ? onFocus : rest.onFocus}
        onBlur={enabled ? onBlur : rest.onBlur}
      >
        {children}
      </Trigger>
      {tooltip ? createPortal(tooltip, portalTarget as HTMLElement) : null}
    </>
  );
}

const Tip = styled.div`
  position: fixed;
  z-index: 1300;
  top: 0;
  left: 0;
  box-sizing: border-box;
  width: max-content;
  max-width: min(320px, calc(100vw - ${VIEWPORT_MARGIN * 2}px));
  margin: 0;
  padding: 6px 10px;
  border: 1px solid var(--v2-rule-strong);
  border-radius: 6px;
  background: var(--v2-surface-raised);
  /* Small, so it does not dim the trigger right under it. */
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.45);
  color: var(--v2-text);
  font-family: var(--v2-font-ui);
  font-size: 0.75rem;
  font-style: normal;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  letter-spacing: normal;
  line-height: 1.45;
  text-align: left;
  text-transform: none;
  white-space: normal;
  overflow-wrap: break-word;

  /* Bridges the gap to the trigger, so the pointer can move onto the tooltip. */
  &::before {
    position: absolute;
    right: 0;
    left: 0;
    height: ${GAP + 2}px;
    content: "";
  }

  &[data-placement="top"]::before {
    top: 100%;
  }

  &[data-placement="bottom"]::before {
    bottom: 100%;
  }
`;
