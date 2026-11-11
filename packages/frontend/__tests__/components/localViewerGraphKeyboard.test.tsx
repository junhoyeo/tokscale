/** @vitest-environment jsdom */

import React from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TokenGraph2D, moveGraphCell } from "@/components/TokenGraph2D";
import { getPalette } from "@/lib/themes";
import { localExport } from "../support/localExport";

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("2D graph keyboard moves", () => {
  it("steps a week with Left and Right, a day with Up and Down, and stops at the edges", () => {
    expect(moveGraphCell([10, 3], "ArrowLeft", 53)).toEqual([9, 3]);
    expect(moveGraphCell([10, 3], "ArrowRight", 53)).toEqual([11, 3]);
    expect(moveGraphCell([10, 3], "ArrowUp", 53)).toEqual([10, 2]);
    expect(moveGraphCell([10, 3], "ArrowDown", 53)).toEqual([10, 4]);
    expect(moveGraphCell([10, 3], "Home", 53)).toEqual([0, 3]);
    expect(moveGraphCell([10, 3], "End", 53)).toEqual([52, 3]);
    expect(moveGraphCell([0, 0], "ArrowLeft", 53)).toEqual([0, 0]);
    expect(moveGraphCell([0, 0], "ArrowUp", 53)).toEqual([0, 0]);
    expect(moveGraphCell([52, 6], "ArrowRight", 53)).toEqual([52, 6]);
    expect(moveGraphCell([52, 6], "ArrowDown", 53)).toEqual([52, 6]);
    expect(moveGraphCell([10, 3], "a", 53)).toBeNull();
  });
});

describe("2D graph keyboard use", () => {
  function setup(selectedDate: string | null = "2026-03-02") {
    const onDayHover = vi.fn();
    const onDayClick = vi.fn();
    const contributions = localExport().contributions.filter((day) => day.date.startsWith("2026"));
    render(
      <TokenGraph2D
        contributions={contributions}
        palette={getPalette("blue")}
        year="2026"
        selectedDate={selectedDate}
        onDayHover={onDayHover}
        onDayClick={onDayClick}
      />,
    );
    const graph = screen.getByRole("application", { name: "Daily token usage in 2026" });
    return { graph, onDayHover, onDayClick };
  }

  it("is focusable and explains its keys", () => {
    const { graph } = setup();
    expect(graph.tabIndex).toBe(0);
    const instructions = document.getElementById(graph.getAttribute("aria-describedby") ?? "");
    expect(instructions?.textContent).toContain("arrow keys");
  });

  it("starts on the selected day, reads each day out, and selects with Enter", () => {
    const { graph, onDayHover, onDayClick } = setup();
    const live = document.querySelector("[aria-live=polite]");

    fireEvent.keyDown(graph, { key: "ArrowRight" });
    expect(live?.textContent).toBe("Monday, March 2, 2026: 2.0M tokens, $39.00, 184 messages, shown in the breakdown");
    expect(onDayHover).toHaveBeenLastCalledWith(expect.objectContaining({ date: "2026-03-02" }), expect.any(Object));

    fireEvent.keyDown(graph, { key: "ArrowDown" });
    expect(live?.textContent).toBe("Tuesday, March 3, 2026: 122.0K tokens, $4.50, 21 messages");
    fireEvent.keyDown(graph, { key: "Enter" });
    expect(onDayClick).toHaveBeenCalledWith(expect.objectContaining({ date: "2026-03-03" }));

    fireEvent.keyDown(graph, { key: "ArrowDown" });
    expect(live?.textContent).toBe("Wednesday, March 4, 2026: no usage");
    expect(onDayHover).toHaveBeenLastCalledWith(null, null);
    fireEvent.keyDown(graph, { key: "Enter" });
    expect(onDayClick).toHaveBeenCalledTimes(1);
  });

  it("hides the tooltip on Escape and clears the ring on blur", () => {
    const { graph, onDayHover } = setup();
    const live = document.querySelector("[aria-live=polite]");
    fireEvent.keyDown(graph, { key: "ArrowLeft" });
    fireEvent.keyDown(graph, { key: "Escape" });
    expect(onDayHover).toHaveBeenLastCalledWith(null, null);
    fireEvent.blur(graph);
    expect(live?.textContent).toBe("");
  });

  it("starts on the latest day with data when no day is selected", () => {
    const { graph } = setup(null);
    fireEvent.keyDown(graph, { key: "Enter" });
    expect(document.querySelector("[aria-live=polite]")?.textContent).toContain("Tuesday, March 3, 2026");
  });
});


describe("2D graph scroll anchor", () => {
  let width: number;
  let resize: ResizeObserverCallback;

  beforeEach(() => {
    width = 321;
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
      return this.tagName === "DIV" ? width : 0;
    });
    vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockReturnValue(700);
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: ResizeObserverCallback) { resize = callback; }
      observe() {}
      disconnect() {}
    });
  });

  function graph(contributions = localExport().contributions, selectedDate: string | null = null, year = "2026") {
    return <TokenGraph2D contributions={contributions} palette={getPalette("blue")} year={year}
      selectedDate={selectedDate} onDayHover={() => {}} onDayClick={() => {}} />;
  }

  const scroller = () => screen.getByRole("application").parentElement!;
  const resizeTo = (next: number) => {
    width = next;
    act(() => resize([{ contentRect: { width } } as ResizeObserverEntry], {} as ResizeObserver));
  };

  it("keeps early-year exports visible instead of scrolling to empty year-end weeks", () => {
    render(graph());
    expect(scroller().scrollLeft).toBe(0);
  });

  it("reveals the latest active week, ignoring later zero-value entries and input order", () => {
    const source = localExport().contributions[0];
    const days = [
      { ...source, date: "2026-12-30", totals: { tokens: 0, cost: 0, messages: 0 } },
      { ...source, date: "2026-10-20" },
      { ...source, date: "2026-03-02" },
    ];
    render(graph(days));
    // October 20 is in week 42: leave room for its cell and month label at the right edge.
    expect(scroller().scrollLeft).toBe(32 + 42 * 12 + 12 + 24 - 321);
  });

  it("prefers a selected day and repositions for a new selection", () => {
    const source = localExport().contributions[0];
    const days = [{ ...source, date: "2026-03-02" }, { ...source, date: "2026-10-20" }];
    const { rerender } = render(graph(days));
    expect(scroller().scrollLeft).toBeGreaterThan(0);
    rerender(graph(days, "2026-03-02"));
    expect(scroller().scrollLeft).toBe(9 * 12);
  });

  it("preserves intentional scrolling during narrow-layout resizes and palette/data rerenders", () => {
    const { rerender } = render(graph());
    scroller().scrollLeft = 260;
    resizeTo(380);
    rerender(graph([...localExport().contributions]));
    expect(scroller().scrollLeft).toBe(260);
  });

  it("reveals activity when a wide layout first becomes scrollable", () => {
    width = 900;
    const source = localExport().contributions[0];
    render(graph([{ ...source, date: "2026-10-20" }]));
    expect(scroller().scrollLeft).toBe(0);
    resizeTo(321);
    expect(scroller().scrollLeft).toBeGreaterThan(0);
  });

  it("handles old exports and resets to the start when the year has no activity", () => {
    const source = localExport().contributions[0];
    const { rerender } = render(graph([{ ...source, date: "2020-10-20" }], null, "2020"));
    expect(scroller().scrollLeft).toBeGreaterThan(0);
    rerender(graph([], null, "2021"));
    expect(scroller().scrollLeft).toBe(0);
  });
});
