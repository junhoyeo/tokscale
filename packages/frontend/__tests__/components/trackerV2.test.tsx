/** @vitest-environment jsdom */

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SupportedAgent } from "@/components/home/agents";
import { CopyCommand } from "@/components/home/CopyCommand";
import { FlowDiagram } from "@/app/(main)/tracker/FlowDiagram";
import { TrackerPageClient } from "@/app/(main)/tracker/TrackerPageClient";
import { TUI_VIEWS, TuiShowcase } from "@/app/(main)/tracker/TuiShowcase";

const agents: SupportedAgent[] = Array.from({ length: 59 }, (_, i) => ({
  key: `agent-${i}`,
  name: ["Claude Code", "Codex CLI", "Cursor"][i] ?? `Agent ${i}`,
  logoUrl: `/assets/v2/agents/agent-${i}.webp`,
}));

function stubBrowser({ reducedMotion = false } = {}) {
  vi.stubGlobal(
    "matchMedia",
    vi.fn((query: string) => ({
      matches: reducedMotion && query.includes("reduce"),
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      constructor(private callback: IntersectionObserverCallback) {}
      observe() {
        this.callback([{ isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
      }
      disconnect() {}
    },
  );
}

describe("tracker flow diagram", () => {
  it("keeps the logs and the CLI inside the machine and sends only counts", () => {
    const html = renderToStaticMarkup(<FlowDiagram agents={agents} />);
    expect(html).toContain("Your Machine");
    expect(html).toContain("Claude Code, Codex CLI, Cursor and 56 more.");
    expect(html).toContain("+55");
    expect(html).toContain("tokscale submit");
    expect(html).toContain("sends counts, never content");
    for (const command of ["bunx tokscale@latest", "tokscale login", "tokscale autosubmit enable"]) {
      expect(html).toContain(command);
    }
  });
});

describe("tracker page", () => {
  it("shows all-time leaderboard totals under the terminal", () => {
    const html = renderToStaticMarkup(
      <TrackerPageClient agents={agents} stats={{ rankedUsers: 298, tokens: 22_443_000_000_000 }} />,
    );
    expect(html).toMatch(/<b>298<\/b> people on the leaderboard/);
    expect(html).toMatch(/<b>22\.4T<\/b> tokens, all time/);
    expect(html).toMatch(/<b>59<\/b> agents supported/);
    expect(html).toContain("/assets/v2/tui/tui-overview.webp");
    expect(html).toContain("Counts leave your machine");
  });

  it("drops the leaderboard numbers when they are unavailable", () => {
    const html = renderToStaticMarkup(<TrackerPageClient agents={agents} stats={null} />);
    expect(html).not.toContain("people on the leaderboard");
    expect(html).toMatch(/<b>59<\/b> agents supported/);
  });

  it("server-renders the whole typed command", () => {
    const html = renderToStaticMarkup(<CopyCommand command="bunx tokscale@latest" typed />);
    expect(html).toContain("bunx tokscale@latest");
    expect(html).toContain("--chars:20");
  });
});

describe("terminal UI tour", () => {
  beforeEach(() => stubBrowser());
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  const selected = () => screen.getByRole("tab", { selected: true });

  it("advances when a view's time is up", () => {
    render(<TuiShowcase />);
    expect(selected()).toHaveProperty("id", "tui-tab-overview");
    const progress = selected().lastElementChild as Element;
    fireEvent.animationEnd(progress);
    expect(selected()).toHaveProperty("id", `tui-tab-${TUI_VIEWS[1].id}`);
    expect(screen.getByRole("button", { name: /pause tour/i })).toBeTruthy();
  });

  it("stops the tour once someone picks a view, and supports the arrow keys", () => {
    render(<TuiShowcase />);
    fireEvent.click(screen.getByRole("tab", { name: "Daily" }));
    expect(selected()).toHaveProperty("id", "tui-tab-daily");
    expect(screen.getByRole("button", { name: /play tour/i })).toBeTruthy();
    fireEvent.keyDown(screen.getByRole("tablist"), { key: "ArrowDown" });
    expect(selected()).toHaveProperty("id", "tui-tab-hourly");
  });


  it("plays with the rotation toggle focused but still pauses for content hover and focus", () => {
    const { container } = render(<TuiShowcase />);
    const running = () => getComputedStyle(selected().lastElementChild!).animationPlayState;
    const toggle = screen.getByRole("button", { name: /pause tour/i });
    act(() => toggle.focus());
    fireEvent.mouseOver(toggle);
    fireEvent.click(toggle);
    expect(toggle.textContent).toContain("Play tour");
    fireEvent.click(toggle);
    expect(document.activeElement).toBe(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    expect(running()).toBe("running");

    fireEvent.mouseOver(screen.getByRole("tabpanel"));
    expect(running()).toBe("paused");
    fireEvent.mouseLeave(container.firstElementChild!);
    expect(running()).toBe("running");
    act(() => selected().focus());
    expect(running()).toBe("paused");
    act(() => toggle.focus());
    expect(running()).toBe("running");
  });

  it("does not tour under reduced motion", () => {
    vi.unstubAllGlobals();
    stubBrowser({ reducedMotion: true });
    render(<TuiShowcase />);
    expect(screen.queryByRole("button", { name: /tour/i })).toBeNull();
  });
});
