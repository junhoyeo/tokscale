"use client";

/**
 * Select: the site's one replacement for native <select> elements.
 *
 * It follows the WAI-ARIA APG "select-only combobox" pattern. The trigger is a
 * <button role="combobox"> that keeps DOM focus while the listbox is open, and
 * aria-activedescendant points at the active option. The listbox is portaled to
 * <body> with fixed positioning, so overflow or containment on an ancestor
 * cannot clip it. A hidden input carries the value, so GET forms still submit.
 *
 * Call sites tune size and colour with CSS custom properties, set on the
 * trigger through `className` or on any ancestor. The base styles only read
 * them, so there is no specificity fight:
 *   --select-height        minimum trigger height (default 36px)
 *   --select-font-size     trigger text size (default 0.875rem)
 *   --select-padding-x     left padding before the text (default 12px)
 *   --select-background    trigger background (default var(--v2-surface))
 *   --select-border-color  trigger border (default var(--v2-rule-strong))
 *   --select-radius        corner radius (default 2px)
 *   --select-width         trigger width (default auto: the widest option)
 *   --select-max-width     trigger max width (default 100%)
 * The chevron always sits 10px from the right edge and the text truncates
 * before it.
 */
import { useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import styled, { css, keyframes } from "styled-components";

export interface SelectOption {
  value: string;
  label: ReactNode;
  /** Text for type-ahead when `label` is not a plain string. */
  textValue?: string;
  disabled?: boolean;
}

export interface SelectOptionGroup {
  label: string;
  options: readonly SelectOption[];
}

export type SelectItem = SelectOption | SelectOptionGroup;

export interface SelectProps {
  options: readonly SelectItem[];
  /** Controlled value. Leave undefined and use `defaultValue` for an uncontrolled select. */
  value?: string;
  defaultValue?: string;
  onChange?: (value: string) => void;
  /** Renders a hidden input with this name, so the value submits with its form. */
  name?: string;
  /** Id of the trigger, for an external <label htmlFor>. */
  id?: string;
  /** Applied to the trigger. */
  className?: string;
  disabled?: boolean;
  /** Shown when the value matches no option. */
  placeholder?: ReactNode;
  /** Inline label before the value, as in "Display 30d average". Decorative: name the select with a label. */
  prefix?: ReactNode;
  /** "field" is a bordered box. "inline" reads as underlined text, for use inside a heading row. */
  variant?: "field" | "inline";
  title?: string;
  "aria-label"?: string;
  "aria-labelledby"?: string;
  "aria-describedby"?: string;
}

interface FlatOption extends SelectOption {
  index: number;
  text: string;
}

interface Section {
  key: string;
  label: string | null;
  options: FlatOption[];
}

const POPUP_MAX_HEIGHT = 320;
const POPUP_MIN_HEIGHT = 96;
const VIEWPORT_MARGIN = 8;
const POPUP_GAP = 4;
const PAGE_JUMP = 10;
const TYPEAHEAD_RESET_MS = 500;

function isGroup(item: SelectItem): item is SelectOptionGroup {
  return "options" in item;
}

function optionText(option: SelectOption): string {
  if (option.textValue !== undefined) return option.textValue;
  if (typeof option.label === "string" || typeof option.label === "number") return String(option.label);
  return option.value;
}

function flattenItems(items: readonly SelectItem[]): { flat: FlatOption[]; sections: Section[] } {
  const flat: FlatOption[] = [];
  const sections: Section[] = [];
  items.forEach((item, itemIndex) => {
    const add = (option: SelectOption): FlatOption => {
      const entry = { ...option, index: flat.length, text: optionText(option) };
      flat.push(entry);
      return entry;
    };
    if (isGroup(item)) {
      sections.push({ key: `group-${itemIndex}`, label: item.label, options: item.options.map(add) });
      return;
    }
    const last = sections[sections.length - 1];
    if (last && last.label === null) last.options.push(add(item));
    else sections.push({ key: `options-${itemIndex}`, label: null, options: [add(item)] });
  });
  return { flat, sections };
}

/** The nearest enabled option from `start`, stepping by `step`; -1 when none. */
function enabledFrom(flat: readonly FlatOption[], start: number, step: 1 | -1): number {
  for (let index = start; index >= 0 && index < flat.length; index += step) {
    if (!flat[index].disabled) return index;
  }
  return -1;
}

/** Type-ahead match, APG style: one repeated letter cycles, a longer string refines. */
function matchTypeahead(flat: readonly FlatOption[], search: string, from: number): number {
  const repeated = search.length > 1 && Array.from(search).every((char) => char === search[0]);
  const needle = repeated ? search[0] : search;
  const start = from < 0 ? 0 : from;
  const offset = needle.length === 1 ? 1 : 0;
  for (let step = 0; step < flat.length; step += 1) {
    const option = flat[(start + offset + step) % flat.length];
    if (!option.disabled && option.text.toLowerCase().startsWith(needle)) return option.index;
  }
  return -1;
}

/** Fixed position under the trigger, flipping above when there is more room there. */
function placePopup(trigger: HTMLElement, list: HTMLElement): void {
  const rect = trigger.getBoundingClientRect();
  const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
  const viewportHeight = document.documentElement.clientHeight || window.innerHeight;
  const triggerStyle = window.getComputedStyle(trigger);
  const fontSize = Number.parseFloat(triggerStyle.fontSize) || 14;
  const paddingLeft = Number.parseFloat(triggerStyle.paddingLeft) || 0;

  list.style.fontSize = `${Math.min(16, Math.max(13, fontSize))}px`;
  // Line the option text up with the trigger text (listbox border 1px + padding 4px).
  list.style.setProperty("--select-option-padding-x", `${Math.max(8, Math.round(paddingLeft - 4))}px`);
  list.style.minWidth = `${Math.round(rect.width)}px`;
  list.style.maxWidth = `${Math.max(0, viewportWidth - VIEWPORT_MARGIN * 2)}px`;
  list.style.maxHeight = "none";

  const natural = list.scrollHeight + 2;
  const below = viewportHeight - rect.bottom - POPUP_GAP - VIEWPORT_MARGIN;
  const above = rect.top - POPUP_GAP - VIEWPORT_MARGIN;
  const wanted = Math.min(natural, POPUP_MAX_HEIGHT);
  const placeAbove = below < wanted && above > below;
  const room = placeAbove ? above : below;
  list.style.maxHeight = `${Math.max(POPUP_MIN_HEIGHT, Math.min(POPUP_MAX_HEIGHT, room))}px`;

  if (placeAbove) {
    list.style.top = "auto";
    list.style.bottom = `${Math.round(viewportHeight - rect.top + POPUP_GAP)}px`;
  } else {
    list.style.bottom = "auto";
    list.style.top = `${Math.round(rect.bottom + POPUP_GAP)}px`;
  }
  const width = list.offsetWidth;
  const left = Math.min(rect.left, viewportWidth - VIEWPORT_MARGIN - width);
  list.style.left = `${Math.round(Math.max(VIEWPORT_MARGIN, left))}px`;
  list.dataset.placement = placeAbove ? "top" : "bottom";
}

export function Select({
  options,
  value,
  defaultValue,
  onChange,
  name,
  id,
  className,
  disabled = false,
  placeholder,
  prefix,
  variant = "field",
  title,
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledby,
  "aria-describedby": ariaDescribedby,
}: SelectProps) {
  const { flat, sections } = useMemo(() => flattenItems(options), [options]);
  // Ids from useId appear only while the list is open, so they never reach server HTML: a page whose
  // server and client trees drift would otherwise leave stale ids that React does not patch.
  const reactId = useId();
  const listboxId = `${reactId}listbox`;
  const optionId = (index: number) => `${reactId}option-${index}`;

  const [internalValue, setInternalValue] = useState(() => defaultValue ?? flat[enabledFrom(flat, 0, 1)]?.value ?? "");
  const [openState, setOpenState] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [portalTarget, setPortalTarget] = useState<HTMLElement | null>(null);
  const [listLabel, setListLabel] = useState<string | undefined>(undefined);

  const triggerRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const hiddenRef = useRef<HTMLInputElement>(null);
  const searchRef = useRef<{ text: string; timer: ReturnType<typeof setTimeout> | null }>({ text: "", timer: null });
  const scrollModeRef = useRef<"center" | "nearest" | null>(null);
  const suppressClickRef = useRef(false);
  const pressingRef = useRef(false);

  const open = openState && !disabled && portalTarget !== null;
  const current = value !== undefined ? value : internalValue;
  const selectedIndex = flat.findIndex((option) => option.value === current);
  const selected = selectedIndex >= 0 ? flat[selectedIndex] : null;
  const active = activeIndex >= 0 && activeIndex < flat.length ? activeIndex : -1;
  const firstEnabled = enabledFrom(flat, 0, 1);
  const lastEnabled = enabledFrom(flat, flat.length - 1, -1);

  const clearSearch = () => {
    const search = searchRef.current;
    if (search.timer) clearTimeout(search.timer);
    search.text = "";
    search.timer = null;
  };

  const openList = (index?: number) => {
    const trigger = triggerRef.current;
    if (!trigger || disabled) return;
    const fallback = selectedIndex >= 0 && !flat[selectedIndex].disabled ? selectedIndex : firstEnabled;
    scrollModeRef.current = "center";
    setActiveIndex(index !== undefined && index >= 0 ? index : fallback);
    setPortalTarget(trigger.closest("dialog") ?? document.body);
    if (!ariaLabel && !ariaLabelledby) {
      const text = Array.from(trigger.labels ?? [])
        .map((label) => label.textContent?.trim() ?? "")
        .filter(Boolean)
        .join(" ");
      setListLabel(text || undefined);
    }
    setOpenState(true);
  };

  const close = () => {
    clearSearch();
    setOpenState(false);
  };

  const moveTo = (index: number) => {
    if (index < 0) return;
    scrollModeRef.current = "nearest";
    setActiveIndex(index);
  };

  const commit = (index: number) => {
    const option = flat[index];
    if (!option || option.disabled || option.value === current) return;
    if (value === undefined) setInternalValue(option.value);
    // Update the hidden input now, so a handler that reads the form in onChange sees the new value.
    if (hiddenRef.current) hiddenRef.current.value = option.value;
    onChange?.(option.value);
  };

  const typeahead = (char: string, from: number): number => {
    const search = searchRef.current;
    if (search.timer) clearTimeout(search.timer);
    search.text += char.toLowerCase();
    search.timer = setTimeout(clearSearch, TYPEAHEAD_RESET_MS);
    const match = matchTypeahead(flat, search.text, from);
    if (match < 0) clearSearch();
    return match;
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) return;
    const { key, altKey, ctrlKey, metaKey } = event;
    const printable = key.length === 1 && !altKey && !ctrlKey && !metaKey;
    const typing = searchRef.current.text.length > 0;

    if (!open) {
      if (key === "ArrowDown" || key === "ArrowUp" || key === "Enter" || (key === " " && !typing)) {
        event.preventDefault();
        openList();
      } else if (key === "Home" || key === "End") {
        event.preventDefault();
        openList(key === "Home" ? firstEnabled : lastEnabled);
      } else if (printable) {
        event.preventDefault();
        openList(typeahead(key, selectedIndex));
      }
      return;
    }

    switch (key) {
      case "ArrowDown":
        event.preventDefault();
        if (!altKey) moveTo(enabledFrom(flat, active + 1, 1));
        return;
      case "ArrowUp":
        event.preventDefault();
        if (altKey) {
          commit(active);
          close();
        } else {
          moveTo(active < 0 ? lastEnabled : enabledFrom(flat, active - 1, -1));
        }
        return;
      case "Home":
        event.preventDefault();
        moveTo(firstEnabled);
        return;
      case "End":
        event.preventDefault();
        moveTo(lastEnabled);
        return;
      case "PageDown": {
        event.preventDefault();
        const target = Math.min(flat.length - 1, Math.max(active, 0) + PAGE_JUMP);
        const next = enabledFrom(flat, target, 1);
        moveTo(next >= 0 ? next : enabledFrom(flat, target, -1));
        return;
      }
      case "PageUp": {
        event.preventDefault();
        const target = Math.max(0, active - PAGE_JUMP);
        const next = enabledFrom(flat, target, -1);
        moveTo(next >= 0 ? next : enabledFrom(flat, target, 1));
        return;
      }
      case "Enter":
        event.preventDefault();
        commit(active);
        close();
        return;
      case "Escape":
        // Close the listbox only; an enclosing dialog must not see this key.
        event.preventDefault();
        event.stopPropagation();
        close();
        return;
      case "Tab":
        // APG: Tab takes the active option, closes, and lets focus move on.
        commit(active);
        close();
        return;
      default:
        if (key === " " && !typing) {
          event.preventDefault();
          commit(active);
          close();
        } else if (printable) {
          event.preventDefault();
          moveTo(typeahead(key, active));
        }
    }
  };

  // Position the popup before paint, and keep it attached on scroll and resize.
  useLayoutEffect(() => {
    if (!open) return;
    const trigger = triggerRef.current;
    const list = listRef.current;
    if (!trigger || !list) return;
    let frame = 0;
    const update = () => {
      frame = 0;
      placePopup(trigger, list);
    };
    const schedule = (event: Event) => {
      if (event.target instanceof Node && list.contains(event.target)) return;
      if (frame === 0) frame = window.requestAnimationFrame(update);
    };
    update();
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    return () => {
      if (frame !== 0) window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
    };
  }, [open, flat.length]);

  // Keep the active option in view: centred when the list opens, nearest edge after a key.
  useLayoutEffect(() => {
    const mode = scrollModeRef.current;
    const list = listRef.current;
    if (!open || !mode || !list || active < 0) return;
    scrollModeRef.current = null;
    const option = list.querySelectorAll<HTMLElement>('[role="option"]')[active];
    if (!option) return;
    const top = option.offsetTop;
    const bottom = top + option.offsetHeight;
    if (mode === "center") {
      list.scrollTop = Math.max(0, top - (list.clientHeight - option.offsetHeight) / 2);
    } else if (top < list.scrollTop) {
      list.scrollTop = Math.max(0, top - POPUP_GAP);
    } else if (bottom > list.scrollTop + list.clientHeight) {
      list.scrollTop = bottom - list.clientHeight + POPUP_GAP;
    }
  }, [open, active]);

  // A pointer press outside closes the list without a change.
  useLayoutEffect(() => {
    if (!open) return;
    const handlePointerDown = (event: Event) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (triggerRef.current?.contains(target) || listRef.current?.contains(target)) return;
      // A press on this select's <label> forwards a click to the trigger; do not let it reopen the list.
      suppressClickRef.current = true;
      document.addEventListener(
        "click",
        () => {
          window.setTimeout(() => {
            suppressClickRef.current = false;
          }, 0);
        },
        { once: true, capture: true },
      );
      clearSearch();
      setOpenState(false);
    };
    document.addEventListener("pointerdown", handlePointerDown, true);
    return () => document.removeEventListener("pointerdown", handlePointerDown, true);
  }, [open]);

  // Keep the hidden input in step with the rendered value (a controlled parent may refuse a change).
  useLayoutEffect(() => {
    if (hiddenRef.current && hiddenRef.current.value !== current) hiddenRef.current.value = current;
  });

  const listbox = open ? (
    <Listbox
      ref={listRef}
      id={listboxId}
      role="listbox"
      tabIndex={-1}
      aria-labelledby={ariaLabelledby}
      aria-label={ariaLabelledby ? undefined : (ariaLabel ?? listLabel)}
      onMouseDown={(event) => event.preventDefault()}
    >
      {sections.map((section) => {
        const rows = section.options.map((option) => (
          <OptionRow
            key={`${option.index}-${option.value}`}
            id={optionId(option.index)}
            role="option"
            aria-selected={option.index === selectedIndex}
            aria-disabled={option.disabled || undefined}
            data-active={option.index === active || undefined}
            onMouseMove={() => {
              if (!option.disabled && option.index !== active) setActiveIndex(option.index);
            }}
            onClick={() => {
              if (option.disabled) return;
              commit(option.index);
              close();
              triggerRef.current?.focus();
            }}
          >
            <OptionText>{option.label}</OptionText>
            <CheckSlot aria-hidden="true">
              {option.index === selectedIndex ? (
                <svg viewBox="0 0 16 16" width="14" height="14" focusable="false">
                  <path d="M3.5 8.5 6.5 11.5 12.5 4.5" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              ) : null}
            </CheckSlot>
          </OptionRow>
        ));
        if (section.label === null) return rows;
        const labelId = `${reactId}${section.key}`;
        return (
          <Group key={section.key} role="group" aria-labelledby={labelId}>
            <GroupLabel id={labelId} role="presentation">
              {section.label}
            </GroupLabel>
            {rows}
          </Group>
        );
      })}
    </Listbox>
  ) : null;

  return (
    <>
      <Trigger
        ref={triggerRef}
        type="button"
        id={id}
        className={className}
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        aria-activedescendant={open && active >= 0 ? optionId(active) : undefined}
        aria-label={ariaLabel}
        aria-labelledby={ariaLabelledby}
        aria-describedby={ariaDescribedby}
        title={title}
        disabled={disabled}
        data-state={open ? "open" : "closed"}
        $variant={variant}
        onPointerDown={() => {
          suppressClickRef.current = false;
          // Safari does not focus a pressed button and blurs the focused one; ignore that blur.
          pressingRef.current = true;
          window.addEventListener(
            "pointerup",
            () => {
              pressingRef.current = false;
            },
            { once: true, capture: true },
          );
        }}
        onClick={(event) => {
          if (suppressClickRef.current) {
            suppressClickRef.current = false;
            return;
          }
          event.currentTarget.focus();
          if (open) close();
          else openList();
        }}
        onKeyDown={handleKeyDown}
        onKeyUp={(event) => {
          // Firefox clicks a button on Space keyup even when keydown was handled.
          if (event.key === " ") event.preventDefault();
        }}
        onBlur={() => {
          if (open && !pressingRef.current) close();
        }}
      >
        {prefix ? <Prefix aria-hidden="true">{prefix}</Prefix> : null}
        <ValueStack>
          <ValueText>{selected ? selected.label : placeholder}</ValueText>
          {flat.map((option) => (
            <Sizer key={`${option.index}-${option.value}`} aria-hidden="true">
              {option.label}
            </Sizer>
          ))}
        </ValueStack>
        <Chevron viewBox="0 0 12 12" aria-hidden="true" focusable="false" $open={open} $variant={variant}>
          <path d="M2.75 4.5 6 7.75 9.25 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </Chevron>
      </Trigger>
      {name !== undefined ? <input ref={hiddenRef} type="hidden" name={name} value={current} disabled={disabled} /> : null}
      {listbox && portalTarget ? createPortal(listbox, portalTarget) : null}
    </>
  );
}

const popupIn = keyframes`
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
`;

const Trigger = styled.button<{ $variant: "field" | "inline" }>`
  position: relative;
  display: inline-flex;
  box-sizing: border-box;
  width: var(--select-width, auto);
  min-width: 0;
  max-width: var(--select-max-width, 100%);
  min-height: var(--select-height, 36px);
  align-items: center;
  gap: 6px;
  margin: 0;
  padding: 0 28px 0 var(--select-padding-x, 12px);
  border: 1px solid var(--select-border-color, var(--v2-rule-strong)); /* taste-skill-ignore: form control */
  border-radius: var(--select-radius, 2px);
  background: var(--select-background, var(--v2-surface));
  color: var(--v2-text);
  font-family: var(--v2-font-ui);
  font-size: var(--select-font-size, 0.875rem);
  font-weight: 500;
  line-height: 1.25;
  letter-spacing: normal;
  text-align: left;
  text-transform: none;
  white-space: nowrap;
  cursor: pointer;
  user-select: none;
  -webkit-tap-highlight-color: transparent;
  transition:
    border-color 120ms ease,
    color 120ms ease;

  &:hover:not(:disabled),
  &[aria-expanded="true"] {
    border-color: var(--v2-text-subtle);
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }

  @media (pointer: coarse) {
    min-height: max(var(--select-height, 36px), 44px);
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }

  ${({ $variant }) =>
    $variant === "inline" &&
    css`
      min-height: var(--select-height, 1.25rem);
      padding: 0 18px 1px 0;
      border: 0;
      border-bottom: 1px solid var(--v2-rule-strong); /* taste-skill-ignore: form field */
      border-radius: 0;
      background: transparent;
      color: var(--v2-text-muted);

      &:hover:not(:disabled),
      &[aria-expanded="true"] {
        border-bottom-color: var(--v2-accent);
        color: var(--v2-text);
      }

      @media (pointer: coarse) {
        min-height: 44px;
      }
    `}
`;

const Prefix = styled.span`
  flex: 0 0 auto;
  color: var(--v2-text-muted);
  font-weight: 400;
`;

const ValueStack = styled.span`
  display: grid;
  flex: 1 1 auto;
  min-width: 0;
`;

const ValueText = styled.span`
  grid-area: 1 / 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

/** Invisible copies of every label, so the trigger is as wide as its widest option, like a native select. */
const Sizer = styled.span`
  grid-area: 1 / 1;
  height: 0;
  min-width: 0;
  overflow: hidden;
  visibility: hidden;
  white-space: nowrap;
`;

const Chevron = styled.svg<{ $open: boolean; $variant: "field" | "inline" }>`
  position: absolute;
  top: 50%;
  right: ${({ $variant }) => ($variant === "inline" ? "2px" : "10px")};
  width: 12px;
  height: 12px;
  margin-top: -6px;
  color: var(--v2-text-muted);
  pointer-events: none;
  transform: rotate(${({ $open }) => ($open ? "180deg" : "0deg")});
  transition: transform 150ms ease;

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const Listbox = styled.div`
  position: fixed;
  z-index: 1200;
  box-sizing: border-box;
  margin: 0;
  padding: 4px;
  overflow-x: hidden;
  overflow-y: auto;
  overscroll-behavior: contain;
  border: 1px solid var(--v2-rule); /* taste-skill-ignore: floating listbox */
  border-radius: var(--radius, 2px);
  background: var(--v2-surface-raised);
  box-shadow: 0 18px 40px rgba(0, 0, 0, 0.55); /* taste-skill-ignore: floating listbox */
  color: var(--v2-text);
  font-family: var(--v2-font-ui);
  font-size: 0.875rem;
  font-weight: 400;
  line-height: 1.3;
  letter-spacing: normal;
  text-align: left;
  text-transform: none;
  outline: none;
  scrollbar-color: rgba(255, 255, 255, 0.22) transparent;
  scrollbar-width: thin;
  animation: ${popupIn} 120ms ease-out;

  @media (prefers-reduced-motion: reduce) {
    animation: none;
  }
`;

const Group = styled.div`
  & + &,
  [role="option"] + & {
    margin-top: 8px;
  }

  & + [role="option"] {
    margin-top: 4px;
  }
`;

const GroupLabel = styled.div`
  padding: 8px var(--select-option-padding-x, 8px) 4px;
  color: var(--v2-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;
  letter-spacing: normal;
  text-transform: none;
  cursor: default;
`;

const OptionRow = styled.div`
  display: flex;
  min-height: 32px;
  align-items: center;
  gap: 12px;
  padding: 6px 8px 6px var(--select-option-padding-x, 8px);
  border-radius: var(--radius, 2px);
  color: var(--v2-text);
  cursor: pointer;
  white-space: nowrap;

  &[data-active="true"] {
    background: #232323;
  }

  &[aria-selected="true"] {
    font-weight: 500;
  }

  &[aria-disabled="true"] {
    color: var(--v2-text-subtle);
    cursor: not-allowed;
  }

  @media (pointer: coarse) {
    min-height: 44px;
  }

  @media (forced-colors: active) {
    &[data-active="true"] {
      outline: 2px solid Highlight;
      outline-offset: -2px;
    }
  }
`;

const OptionText = styled.span`
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
`;

const CheckSlot = styled.span`
  display: inline-flex;
  flex: 0 0 14px;
  width: 14px;
  height: 14px;
  color: var(--v2-accent);
`;
