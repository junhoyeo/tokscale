/** @vitest-environment jsdom */

import React from "react";
import { readFileSync } from "node:fs";
import path from "node:path";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import LocalClient, { formatExportRange } from "@/app/local/LocalClient";
import { localExport } from "../support/localExport";

// The real bar fetches the session on mount; the shell is what matters here.
vi.mock("@/components/layout/Navigation", () => ({
  Navigation: () => <nav aria-label="Main navigation" />,
}));

beforeEach(() => {
  // jsdom has no canvas; the graphs already skip drawing without a context.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("local viewer page", () => {
  it("sits in the v2 page shell with the navigation, a display heading and the public footer", () => {
    const { container } = render(<LocalClient />);
    expect(container.firstElementChild?.classList.contains("service-page-shell")).toBe(true);
    expect(screen.getByRole("navigation", { name: "Main navigation" })).toBeTruthy();
    expect(screen.getByRole("main").id).toBe("main-content");
    expect(screen.getByRole("heading", { level: 1, name: "Local viewer" })).toBeTruthy();
    expect(screen.getByRole("navigation", { name: "Footer" })).toBeTruthy();
  });

  it("selects the latest active date from an unsorted export, not its last entry", () => {
    const data = localExport();
    data.contributions = [
      { ...data.contributions[1], date: "2026-10-20" },
      { ...data.contributions[1], date: "2026-12-31", totals: { tokens: 0, cost: 0, messages: 0 } },
      data.contributions[2],
    ];
    render(<LocalClient />);
    fireEvent.change(screen.getByRole("textbox", { name: "Export JSON" }), { target: { value: JSON.stringify(data) } });
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));
    expect(within(screen.getByRole("region", { name: "Day breakdown" })).getByRole("heading", { name: "October 20, 2026" })).toBeTruthy();
  });

  it("does not open an arbitrary empty day when an export has no activity", () => {
    const data = localExport();
    data.contributions = data.contributions.map(day => ({ ...day, totals: { tokens: 0, cost: 0, messages: 0 } }));
    render(<LocalClient />);
    fireEvent.change(screen.getByRole("textbox", { name: "Export JSON" }), { target: { value: JSON.stringify(data) } });
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));
    expect(screen.getByRole("application", { name: "Daily token usage in 2026" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Day breakdown" })).toBeNull();
  });

  it("renders an empty export with no year entries without an invalid calendar", () => {
    const data = localExport();
    data.contributions = [];
    data.years = [];
    data.summary = { ...data.summary, totalTokens: 0, totalCost: 0, activeDays: 0, totalDays: 0 };
    render(<LocalClient />);
    fireEvent.change(screen.getByRole("textbox", { name: "Export JSON" }), { target: { value: JSON.stringify(data) } });
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));
    expect(screen.getByRole("application", { name: `Daily token usage in ${new Date().getFullYear()}` })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Day breakdown" })).toBeNull();
  });

  it("renders an unknown future client with its raw id", () => {
    const data = JSON.parse(JSON.stringify(localExport())) as {
      summary: { clients: string[] };
      contributions: Array<{ clients: Array<{ client: string }> }>;
    };
    data.summary.clients = ["future-agent"];
    for (const day of data.contributions) {
      for (const line of day.clients) line.client = "future-agent";
    }

    render(<LocalClient />);
    fireEvent.change(screen.getByRole("textbox", { name: "Export JSON" }), {
      target: { value: JSON.stringify(data) },
    });
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));

    expect(screen.getByLabelText("Loaded export")).toBeTruthy();
    const breakdown = screen.getByRole("region", { name: "Day breakdown" });
    expect(within(breakdown).getByText("future-agent")).toBeTruthy();
  });

  it("shows a loaded export and goes back to the input", () => {
    render(<LocalClient />);
    fireEvent.change(screen.getByRole("textbox", { name: "Export JSON" }), {
      target: { value: JSON.stringify(localExport()) },
    });
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));

    const summary = screen.getByLabelText("Loaded export");
    expect(summary.tagName).toBe("DL");
    expect(summary.textContent).toContain("Dec 30, 2025 – Mar 3, 2026");
    expect(summary.textContent).toContain("$56.00");
    expect(screen.getByRole("heading", { level: 2, name: /tokens used in/ }).textContent).toContain("tokens used in");
    expect(screen.getByRole("combobox", { name: "Color palette" })).toBeTruthy();

    // The latest active day opens in the breakdown, as before.
    const breakdown = screen.getByRole("region", { name: "Day breakdown" });
    expect(within(breakdown).getByRole("heading", { name: "March 3, 2026" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Statistics" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Load different data" }));
    expect(screen.getByRole("textbox", { name: "Export JSON" })).toBeTruthy();
  });
});

describe("bundled sample data", () => {
  it("loads into the viewer in the shape `tokscale graph` prints today", async () => {
    const sample = JSON.parse(readFileSync(path.join(__dirname, "../../public/sample-data.json"), "utf8"));
    expect(Array.isArray(sample.summary.clients)).toBe(true);
    expect(sample.contributions.every((day: { clients?: { client?: string }[] }) => day.clients?.every((line) => line.client))).toBe(true);

    vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, status: 200, json: async () => sample })));
    render(<LocalClient />);
    fireEvent.click(screen.getByRole("button", { name: "Load sample data" }));
    // Parsing and drawing the full sample export can take more than the default 1 s on a busy machine.
    await waitFor(() => expect(screen.getByLabelText("Loaded export")).toBeTruthy(), { timeout: 5000 });
    expect(screen.getByRole("group", { name: "Client filters" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Day breakdown" })).toBeTruthy();
  });
});

describe("export range label", () => {
  it("formats calendar days in UTC and keeps anything else as written", () => {
    expect(formatExportRange({ start: "2025-11-06", end: "2026-10-22" })).toBe("Nov 6, 2025 – Oct 22, 2026");
    expect(formatExportRange({ start: "2026-03-03", end: "2026-03-03" })).toBe("Mar 3, 2026");
    expect(formatExportRange({ start: "yesterday", end: "" })).toBe("yesterday");
    expect(formatExportRange(undefined)).toBe("No dates");
  });
});
