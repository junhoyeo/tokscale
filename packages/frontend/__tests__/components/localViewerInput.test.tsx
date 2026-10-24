/** @vitest-environment jsdom */

import React from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DataInput } from "@/components/DataInput";
import { localExport } from "../support/localExport";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function setup() {
  const onDataLoaded = vi.fn();
  render(<DataInput onDataLoaded={onDataLoaded} />);
  const textarea = screen.getByRole("textbox", { name: "Export JSON" });
  return { onDataLoaded, textarea };
}

describe("local viewer data input", () => {
  it("parses pasted `tokscale graph` JSON", () => {
    const { onDataLoaded, textarea } = setup();
    const parse = screen.getByRole("button", { name: "Parse JSON" });
    expect(parse).toHaveProperty("disabled", true);

    const data = localExport();
    fireEvent.change(textarea, { target: { value: JSON.stringify(data) } });
    expect(parse).toHaveProperty("disabled", false);
    fireEvent.click(parse);
    expect(onDataLoaded).toHaveBeenCalledWith(data);
  });

  it("parses with Ctrl+Enter or Cmd+Enter", () => {
    const { onDataLoaded, textarea } = setup();
    fireEvent.change(textarea, { target: { value: JSON.stringify(localExport()) } });
    fireEvent.keyDown(textarea, { key: "Enter", ctrlKey: true });
    fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
    expect(onDataLoaded).toHaveBeenCalledTimes(2);
  });

  it("reports broken JSON and the wrong shape as alerts on the field", () => {
    const { onDataLoaded, textarea } = setup();
    fireEvent.change(textarea, { target: { value: "{not json" } });
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));
    expect(screen.getByRole("alert").textContent).toMatch(/^Invalid JSON: /);
    expect(textarea.getAttribute("aria-invalid")).toBe("true");
    expect(textarea.getAttribute("aria-describedby")).toContain(screen.getByRole("alert").id);

    fireEvent.change(textarea, { target: { value: JSON.stringify({ meta: {} }) } });
    expect(screen.queryByRole("alert")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));
    expect(screen.getByRole("alert").textContent).toContain("Invalid data format");
    expect(onDataLoaded).not.toHaveBeenCalled();
  });

  it("loads the sample data", async () => {
    const data = localExport();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({ ok: true, status: 200, json: async () => data })),
    );
    const { onDataLoaded } = setup();
    fireEvent.click(screen.getByRole("button", { name: "Load sample data" }));
    await waitFor(() => expect(onDataLoaded).toHaveBeenCalledWith(data));
    expect(fetch).toHaveBeenCalledWith("/sample-data.json");
  });

  it("loads a chosen .json file in the browser", async () => {
    const data = localExport();
    const { onDataLoaded } = setup();
    expect(screen.getByRole("button", { name: "Choose file" })).toBeTruthy();
    const input = document.querySelector<HTMLInputElement>('input[type="file"]');
    expect(input?.accept).toBe(".json,application/json");
    const file = new File([JSON.stringify(data)], "usage.json", { type: "application/json" });
    fireEvent.change(input as HTMLInputElement, { target: { files: [file] } });
    await waitFor(() => expect(onDataLoaded).toHaveBeenCalledWith(data));
  });

  it("loads a dropped export and shows where to drop it while dragging", async () => {
    const data = localExport();
    const { onDataLoaded } = setup();
    const card = screen.getByRole("region", { name: "Load token usage data" });
    const file = new File([JSON.stringify(data)], "usage.json", { type: "application/json" });
    const dataTransfer = { types: ["Files"], files: [file], dropEffect: "none" };

    fireEvent.dragEnter(card, { dataTransfer });
    expect(card.textContent).toContain("Drop the export to load it");
    fireEvent.drop(card, { dataTransfer });
    expect(card.textContent).not.toContain("Drop the export to load it");
    await waitFor(() => expect(onDataLoaded).toHaveBeenCalledWith(data));
  });

  it("refuses a dropped file that is not JSON", async () => {
    const { onDataLoaded } = setup();
    const card = screen.getByRole("region", { name: "Load token usage data" });
    const file = new File(["not an export"], "notes.txt", { type: "text/plain" });
    fireEvent.drop(card, { dataTransfer: { types: ["Files"], files: [file] } });
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("notes.txt is not a .json file"));
    expect(onDataLoaded).not.toHaveBeenCalled();
  });

  it("explains how to get an export", () => {
    setup();
    const steps = screen.getByRole("region", { name: "How to get your data" });
    expect(steps.textContent).toContain("bunx tokscale");
    expect(steps.textContent).toContain("tokscale graph");
    expect(steps.textContent).toContain("tokscale graph --output usage.json");
  });
});
