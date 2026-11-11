/** @vitest-environment jsdom */

import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { GraphControls, GraphPaletteLegend } from "@/components/GraphControls";
import { colorPalettes, getPalette, getPaletteNames } from "@/lib/themes";
import type { ClientType } from "@/lib/types";

afterEach(() => {
  cleanup();
});

function renderControls(overrides: Partial<React.ComponentProps<typeof GraphControls>> = {}) {
  const props: React.ComponentProps<typeof GraphControls> = {
    view: "2d",
    onViewChange: vi.fn(),
    selectedYear: "2026",
    availableYears: ["2025", "2026"],
    onYearChange: vi.fn(),
    clientFilter: [],
    availableClients: ["claude", "codex"] as ClientType[],
    onClientFilterChange: vi.fn(),
    totalTokens: 2_468_000,
    ...overrides,
  };
  const view = render(<GraphControls {...props} />);
  return { ...view, props };
}

describe("local viewer graph controls", () => {
  it("switches the year with the shared Select instead of a native select", () => {
    const { container, props } = renderControls();
    expect(container.querySelector("select")).toBeNull();
    expect(screen.getByRole("heading", { level: 2 }).textContent).toContain("2.5M tokens used in");

    const year = screen.getByRole("combobox", { name: "Select year" });
    year.focus();
    fireEvent.keyDown(year, { key: "ArrowDown" });
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual(["2025", "2026"]);
    fireEvent.keyDown(year, { key: "ArrowUp" });
    fireEvent.keyDown(year, { key: "Enter" });
    expect(props.onYearChange).toHaveBeenCalledWith("2025");
  });

  it("shows a single year as text", () => {
    renderControls({ availableYears: ["2026"] });
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.getByRole("heading", { level: 2 }).textContent).toContain("tokens used in 2026");
  });

  it("toggles between the 2D and 3D views", () => {
    const { props } = renderControls();
    expect(screen.getByRole("button", { name: "2D view" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "3D view" }).getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(screen.getByRole("button", { name: "3D view" }));
    expect(props.onViewChange).toHaveBeenCalledWith("3d");
  });

  it("filters clients and offers Clear or Show all", () => {
    const { props, rerender } = renderControls();
    expect(screen.getByRole("group", { name: "Client filters" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Filter by Codex CLI" }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "Filter by Codex CLI" }));
    expect(props.onClientFilterChange).toHaveBeenCalledWith(["claude"]);

    rerender(<GraphControls {...props} clientFilter={["claude"] as ClientType[]} />);
    expect(screen.getByRole("button", { name: "Filter by Codex CLI" }).getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(screen.getByRole("button", { name: "Show all clients" }));
    expect(props.onClientFilterChange).toHaveBeenLastCalledWith(["claude", "codex"]);

    rerender(<GraphControls {...props} clientFilter={["claude", "codex"] as ClientType[]} />);
    fireEvent.click(screen.getByRole("button", { name: "Clear all client filters" }));
    expect(props.onClientFilterChange).toHaveBeenLastCalledWith([]);
  });

  it("hides the client filter when the export has one client", () => {
    renderControls({ availableClients: ["claude"] as ClientType[] });
    expect(screen.queryByRole("group", { name: "Client filters" })).toBeNull();
  });
});

describe("local viewer palette and legend", () => {
  it("picks a palette with the shared Select and shows the low-to-high key", () => {
    const onPaletteChange = vi.fn();
    const { container } = render(
      <GraphPaletteLegend paletteName="blue" onPaletteChange={onPaletteChange} palette={getPalette("blue")} />,
    );
    expect(container.querySelector("select")).toBeNull();
    expect(screen.getByRole("img", { name: "Usage intensity, low to high" })).toBeTruthy();

    const trigger = screen.getByRole("combobox", { name: "Color palette" });
    expect(trigger.textContent).toContain("Blue");
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual(
      getPaletteNames().map((name) => colorPalettes[name].name),
    );
    fireEvent.keyDown(trigger, { key: "h" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(onPaletteChange).toHaveBeenCalledWith("halloween");
  });
});
