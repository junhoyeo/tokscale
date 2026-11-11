/** @vitest-environment jsdom */

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { InstantTooltip } from "@/components/ui/InstantTooltip";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  Reflect.deleteProperty(document, "elementFromPoint");
});

function tooltipNode(): HTMLElement | null {
  return document.body.querySelector('[role="tooltip"], [aria-hidden="true"][id$="tooltip"]');
}

/** Gives a jsdom element a box, so the row tracking can tell whether the pointer is over it. */
function giveBox(element: Element, box: { left: number; top: number; right: number; bottom: number }) {
  const rect = { ...box, x: box.left, y: box.top, width: box.right - box.left, height: box.bottom - box.top, toJSON: () => box };
  Object.defineProperty(element, "getClientRects", { value: () => [rect] });
  Object.defineProperty(element, "getBoundingClientRect", { value: () => rect });
}

function pointer(type: string, target: Element, init: { clientX?: number; clientY?: number; relatedTarget?: Element | null } = {}) {
  const event = new MouseEvent(type, { bubbles: type !== "pointerleave", cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(event);
  });
}

describe("InstantTooltip", () => {
  it("renders only the trigger on the server, with no tooltip and no generated id", () => {
    const markup = renderToStaticMarkup(
      <InstantTooltip className="dash" content="Not tracked yet">
        —
      </InstantTooltip>,
    );
    expect(markup).toBe('<span class="dash">—</span>');
  });

  it("opens on hover at once, describes the trigger, and closes when the pointer leaves", () => {
    render(<InstantTooltip content="Not a lead in a tracked round">—</InstantTooltip>);
    const trigger = screen.getByText("—");
    expect(tooltipNode()).toBeNull();

    fireEvent.pointerEnter(trigger);
    const tooltip = screen.getByRole("tooltip");
    expect(tooltip.textContent).toBe("Not a lead in a tracked round");
    expect(trigger.getAttribute("aria-describedby")).toBe(tooltip.id);
    // Portaled out of the trigger, so no overflow container can clip it.
    expect(tooltip.parentElement).toBe(document.body);

    fireEvent.pointerLeave(trigger);
    expect(tooltipNode()).toBeNull();
    expect(trigger.hasAttribute("aria-describedby")).toBe(false);
  });

  it("closes on Escape and stays closed until the pointer leaves", () => {
    render(<InstantTooltip content="No tracked round yet">—</InstantTooltip>);
    const trigger = screen.getByText("—");
    fireEvent.pointerEnter(trigger);
    expect(screen.getByRole("tooltip")).toBeTruthy();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(tooltipNode()).toBeNull();

    fireEvent.pointerLeave(trigger);
    fireEvent.pointerEnter(trigger);
    expect(screen.getByRole("tooltip")).toBeTruthy();
  });

  it("opens on keyboard focus and closes on blur", () => {
    render(
      <InstantTooltip content="Context window in tokens (models.dev)." focusable>
        Context
      </InstantTooltip>,
    );
    const trigger = screen.getByText("Context");
    expect(trigger.getAttribute("tabindex")).toBe("0");

    act(() => trigger.focus());
    expect(screen.getByRole("tooltip").textContent).toBe("Context window in tokens (models.dev).");

    act(() => trigger.blur());
    expect(tooltipNode()).toBeNull();
  });

  it("hides the tooltip from screen readers when the trigger already carries the text", () => {
    render(
      <InstantTooltip content="#1 by tokens, last 30 days" describe={false}>
        #1
      </InstantTooltip>,
    );
    const trigger = screen.getByText("#1");
    fireEvent.pointerEnter(trigger);
    const tooltip = tooltipNode();
    expect(tooltip?.textContent).toBe("#1 by tokens, last 30 days");
    expect(tooltip?.getAttribute("aria-hidden")).toBe("true");
    expect(tooltip?.hasAttribute("role")).toBe(false);
    expect(trigger.hasAttribute("aria-describedby")).toBe(false);
  });

  it("renders as another element and passes its props through", () => {
    render(
      <InstantTooltip as="a" href="https://www.sec.gov/" target="_blank" rel="noopener noreferrer" content="Form 8-K filing">
        8-K
      </InstantTooltip>,
    );
    const link = screen.getByRole("link", { name: "8-K" });
    expect(link.getAttribute("href")).toBe("https://www.sec.gov/");
    expect(link.getAttribute("target")).toBe("_blank");
    fireEvent.pointerDown(link);
    expect(tooltipNode()).toBeNull();
  });

  it("does nothing without content", () => {
    render(<InstantTooltip content={null}>Plain</InstantTooltip>);
    fireEvent.pointerEnter(screen.getByText("Plain"));
    expect(tooltipNode()).toBeNull();
  });

  it("follows the pointer over a row link, where the row's link covers the trigger", () => {
    render(
      <table>
        <tbody>
          <tr data-row-link="">
            <td>
              <a href="#fidelity" data-row-primary="">
                Fidelity
              </a>
            </td>
            <td>
              <InstantTooltip content="Not a lead in a tracked round" describe={false}>
                —
              </InstantTooltip>
            </td>
          </tr>
        </tbody>
      </table>,
    );
    const row = document.querySelector("tr") as HTMLTableRowElement;
    const trigger = screen.getByText("—");
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: vi.fn(() => screen.getByRole("link")) });
    giveBox(trigger, { left: 100, top: 10, right: 110, bottom: 30 });

    pointer("pointermove", row, { clientX: 20, clientY: 20 });
    expect(tooltipNode()).toBeNull();

    pointer("pointermove", row, { clientX: 105, clientY: 20 });
    expect(tooltipNode()?.textContent).toBe("Not a lead in a tracked round");

    pointer("pointermove", row, { clientX: 140, clientY: 20 });
    expect(tooltipNode()).toBeNull();

    pointer("pointermove", row, { clientX: 105, clientY: 20 });
    pointer("pointerleave", row, { relatedTarget: document.body });
    expect(tooltipNode()).toBeNull();
  });

  it("rejects a covered value behind a sticky identity cell but permits the stretched row cover", () => {
    render(<table><tbody><tr data-row-link="">
      <td style={{ position: "sticky", left: 0 }}><a href="#model" data-row-primary="">Model</a></td>
      <td><a href="#model" data-row-cover="" aria-hidden="true" tabIndex={-1} />
        <InstantTooltip content="Active users increased">+87</InstantTooltip>
      </td>
    </tr></tbody></table>);
    const row = document.querySelector("tr")!;
    const trigger = screen.getByText("+87");
    const identity = screen.getByText("Model");
    const cover = document.querySelector("[data-row-cover]")!;
    giveBox(trigger, { left: 100, top: 10, right: 150, bottom: 30 });
    const hit = vi.fn((): Element => identity);
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: hit });

    pointer("pointermove", row, { clientX: 120, clientY: 20 });
    expect(tooltipNode()).toBeNull();
    hit.mockReturnValue(cover);
    pointer("pointermove", row, { clientX: 120, clientY: 20 });
    expect(screen.getByRole("tooltip").textContent).toBe("Active users increased");
    const tooltip = screen.getByRole("tooltip");
    pointer("pointerleave", row, { relatedTarget: tooltip });
    expect(tooltipNode()).toBe(tooltip);
    hit.mockReturnValue(identity);
    pointer("pointerout", tooltip, { clientX: 120, clientY: 20, relatedTarget: identity });
    expect(tooltipNode()).toBeNull();
  });

});
