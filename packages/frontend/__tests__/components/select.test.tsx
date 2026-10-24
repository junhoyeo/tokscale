/** @vitest-environment jsdom */

import React, { useState } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Select, type SelectItem } from "@/components/ui/Select";

const COUNTRIES: SelectItem[] = [
  { value: "", label: "All countries" },
  { value: "CA", label: "Canada" },
  { value: "CN", label: "China" },
  { value: "FR", label: "France", disabled: true },
  { value: "DE", label: "Germany" },
  { value: "GR", label: "Greece" },
  { value: "IN", label: "India" },
  { value: "JP", label: "Japan" },
  { value: "KR", label: "South Korea" },
  { value: "SE", label: "Sweden" },
  { value: "CH", label: "Switzerland" },
  { value: "AE", label: "United Arab Emirates" },
  { value: "GB", label: "United Kingdom" },
  { value: "US", label: "United States" },
];

function setup(props: Partial<React.ComponentProps<typeof Select>> = {}) {
  const onChange = vi.fn();
  render(<Select aria-label="Country" options={COUNTRIES} defaultValue="CN" onChange={onChange} {...props} />);
  const trigger = screen.getByRole("combobox", { name: "Country" });
  trigger.focus();
  return { trigger, onChange };
}

/** A mouse click as the browser sends it: pointerdown, mousedown, then click. */
function press(element: HTMLElement) {
  fireEvent.pointerDown(element);
  fireEvent.mouseDown(element);
  fireEvent.click(element);
}

function activeOption(trigger: HTMLElement): HTMLElement | null {
  const id = trigger.getAttribute("aria-activedescendant");
  return id ? document.getElementById(id) : null;
}

afterEach(() => {
  cleanup();
});

describe("Select", () => {
  it("renders a closed select-only combobox", () => {
    const { trigger } = setup();
    expect(trigger.tagName).toBe("BUTTON");
    expect(trigger.getAttribute("type")).toBe("button");
    expect(trigger.getAttribute("aria-haspopup")).toBe("listbox");
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    // The listbox exists only while open, so the closed trigger does not point at it.
    expect(trigger.hasAttribute("aria-controls")).toBe(false);
    expect(trigger.hasAttribute("aria-activedescendant")).toBe(false);
    expect(trigger.textContent).toContain("China");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("keeps generated ids out of server HTML", () => {
    const markup = renderToStaticMarkup(<Select aria-label="Country" name="country" options={COUNTRIES} defaultValue="CN" />);
    expect(markup).toContain('role="combobox"');
    expect(markup).toContain('<input type="hidden" name="country" value="CN"/>');
    expect(markup).not.toMatch(/\sid="/);
    expect(markup).not.toContain("aria-controls");
    expect(markup).not.toContain("aria-activedescendant");
  });

  it("opens with ArrowDown on the selected option and keeps focus on the trigger", () => {
    const { trigger } = setup();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });

    const listbox = screen.getByRole("listbox");
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    expect(trigger.getAttribute("aria-controls")).toBe(listbox.id);
    expect(activeOption(trigger)?.textContent).toBe("China");
    expect(screen.getByRole("option", { name: "China" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("option", { name: "Canada" }).getAttribute("aria-selected")).toBe("false");
    expect(document.activeElement).toBe(trigger);
  });

  it.each(["Enter", " ", "ArrowUp"])("opens with %j", (key) => {
    const { trigger, onChange } = setup();
    fireEvent.keyDown(trigger, { key });
    expect(screen.getByRole("listbox")).toBeTruthy();
    expect(activeOption(trigger)?.textContent).toBe("China");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("opens with Alt+ArrowDown and opens on Home and End at the ends", () => {
    const { trigger } = setup();
    fireEvent.keyDown(trigger, { key: "ArrowDown", altKey: true });
    expect(activeOption(trigger)?.textContent).toBe("China");
    fireEvent.keyDown(trigger, { key: "Escape" });

    fireEvent.keyDown(trigger, { key: "End" });
    expect(activeOption(trigger)?.textContent).toBe("United States");
    fireEvent.keyDown(trigger, { key: "Escape" });

    fireEvent.keyDown(trigger, { key: "Home" });
    expect(activeOption(trigger)?.textContent).toBe("All countries");
  });

  it("moves with the arrow keys, skips disabled options and stops at the ends", () => {
    const { trigger } = setup();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    expect(activeOption(trigger)?.textContent).toBe("Germany");
    fireEvent.keyDown(trigger, { key: "ArrowUp" });
    expect(activeOption(trigger)?.textContent).toBe("China");

    fireEvent.keyDown(trigger, { key: "Home" });
    fireEvent.keyDown(trigger, { key: "ArrowUp" });
    expect(activeOption(trigger)?.textContent).toBe("All countries");

    fireEvent.keyDown(trigger, { key: "End" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    expect(activeOption(trigger)?.textContent).toBe("United States");
  });

  it("jumps ten options with PageDown and PageUp", () => {
    const { trigger } = setup({ defaultValue: "" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "PageDown" });
    expect(activeOption(trigger)?.textContent).toBe("Switzerland");
    fireEvent.keyDown(trigger, { key: "PageDown" });
    expect(activeOption(trigger)?.textContent).toBe("United States");
    // Ten up lands on France, which is disabled, so the jump goes on to China.
    fireEvent.keyDown(trigger, { key: "PageUp" });
    expect(activeOption(trigger)?.textContent).toBe("China");
    fireEvent.keyDown(trigger, { key: "PageUp" });
    expect(activeOption(trigger)?.textContent).toBe("All countries");
  });

  it("selects with Enter, closes, and keeps focus on the trigger", () => {
    const { trigger, onChange } = setup();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "Enter" });

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith("DE");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(trigger.textContent).toContain("Germany");
    expect(document.activeElement).toBe(trigger);
  });

  it("selects with Space", () => {
    const { trigger, onChange } = setup();
    fireEvent.keyDown(trigger, { key: " " });
    fireEvent.keyDown(trigger, { key: "ArrowUp" });
    fireEvent.keyDown(trigger, { key: " " });
    expect(onChange).toHaveBeenCalledWith("CA");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("does not call onChange when the selected option is chosen again", () => {
    const { trigger, onChange } = setup();
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("closes on Escape without a change, keeps focus, and keeps the key from outer handlers", () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    try {
      const { trigger, onChange } = setup();
      fireEvent.keyDown(trigger, { key: "ArrowDown" });
      fireEvent.keyDown(trigger, { key: "ArrowDown" });
      fireEvent.keyDown(trigger, { key: "Escape" });

      expect(screen.queryByRole("listbox")).toBeNull();
      expect(onChange).not.toHaveBeenCalled();
      expect(trigger.textContent).toContain("China");
      expect(document.activeElement).toBe(trigger);
      expect(outer.mock.calls.some(([event]) => (event as KeyboardEvent).key === "Escape")).toBe(false);

      // Closed, Escape is not ours: a dialog around the select may close.
      fireEvent.keyDown(trigger, { key: "Escape" });
      expect(outer.mock.calls.some(([event]) => (event as KeyboardEvent).key === "Escape")).toBe(true);
    } finally {
      window.removeEventListener("keydown", outer);
    }
  });

  it("selects the active option and closes on Tab", () => {
    const { trigger, onChange } = setup();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "Tab" });
    expect(onChange).toHaveBeenCalledWith("DE");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  describe("type-ahead", () => {
    it("cycles through options that start with a repeated letter", () => {
      vi.useFakeTimers();
      try {
        const { trigger } = setup({ defaultValue: "" });
        fireEvent.keyDown(trigger, { key: "ArrowDown" });
        fireEvent.keyDown(trigger, { key: "s" });
        expect(activeOption(trigger)?.textContent).toBe("South Korea");
        fireEvent.keyDown(trigger, { key: "s" });
        expect(activeOption(trigger)?.textContent).toBe("Sweden");
        fireEvent.keyDown(trigger, { key: "s" });
        expect(activeOption(trigger)?.textContent).toBe("Switzerland");
        fireEvent.keyDown(trigger, { key: "s" });
        expect(activeOption(trigger)?.textContent).toBe("South Korea");
      } finally {
        vi.useRealTimers();
      }
    });

    it("matches typed prefixes, including spaces, and starts fresh after a pause", () => {
      vi.useFakeTimers();
      try {
        const { trigger } = setup({ defaultValue: "" });
        fireEvent.keyDown(trigger, { key: "ArrowDown" });
        for (const key of ["u", "n", "i", "t", "e", "d", " ", "s"]) fireEvent.keyDown(trigger, { key });
        expect(activeOption(trigger)?.textContent).toBe("United States");
        expect(screen.getByRole("listbox")).toBeTruthy();

        vi.advanceTimersByTime(600);
        fireEvent.keyDown(trigger, { key: "g" });
        expect(activeOption(trigger)?.textContent).toBe("Germany");
        fireEvent.keyDown(trigger, { key: "r" });
        expect(activeOption(trigger)?.textContent).toBe("Greece");
      } finally {
        vi.useRealTimers();
      }
    });

    it("skips disabled options and opens a closed select on the match", () => {
      vi.useFakeTimers();
      try {
        const { trigger, onChange } = setup();
        fireEvent.keyDown(trigger, { key: "f" });
        expect(screen.getByRole("listbox")).toBeTruthy();
        // France is disabled, so the active option stays on the selection.
        expect(activeOption(trigger)?.textContent).toBe("China");

        vi.advanceTimersByTime(600);
        fireEvent.keyDown(trigger, { key: "j" });
        expect(activeOption(trigger)?.textContent).toBe("Japan");
        fireEvent.keyDown(trigger, { key: "Enter" });
        expect(onChange).toHaveBeenCalledWith("JP");
      } finally {
        vi.useRealTimers();
      }
    });
  });

  it("selects with a click and closes on a press outside without a change", () => {
    const { trigger, onChange } = setup();
    press(trigger);
    expect(screen.getByRole("listbox")).toBeTruthy();
    press(screen.getByRole("option", { name: "Japan" }));
    expect(onChange).toHaveBeenCalledWith("JP");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(document.activeElement).toBe(trigger);

    press(trigger);
    fireEvent.mouseMove(screen.getByRole("option", { name: "India" }));
    expect(activeOption(trigger)?.textContent).toBe("India");
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onChange).toHaveBeenCalledTimes(1);

    press(trigger);
    press(screen.getByRole("option", { name: "France" }));
    expect(screen.getByRole("listbox")).toBeTruthy();
    expect(onChange).toHaveBeenCalledTimes(1);

    // A second press on the trigger closes the list.
    press(trigger);
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("submits through a hidden input that holds the new value inside onChange", () => {
    const seen: Array<string | null> = [];
    function Form() {
      return (
        <form aria-label="filters">
          <Select
            aria-label="Country"
            name="country"
            options={COUNTRIES}
            defaultValue=""
            onChange={() => {
              const form = document.querySelector("form") as HTMLFormElement;
              seen.push(new FormData(form).get("country") as string | null);
            }}
          />
        </form>
      );
    }
    render(<Form />);
    const form = screen.getByRole("form", { name: "filters" }) as HTMLFormElement;
    const hidden = form.querySelector('input[type="hidden"][name="country"]') as HTMLInputElement;
    expect(hidden.value).toBe("");

    const trigger = screen.getByRole("combobox", { name: "Country" });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "End" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(seen).toEqual(["US"]);
    expect(hidden.value).toBe("US");
    expect(new FormData(form).get("country")).toBe("US");
  });

  it("follows a controlled value and reports changes through onChange", () => {
    const changes: string[] = [];
    function Controlled() {
      const [value, setValue] = useState("KR");
      return (
        <Select
          aria-label="Country"
          name="country"
          options={COUNTRIES}
          value={value}
          onChange={(next) => {
            changes.push(next);
            setValue(next);
          }}
        />
      );
    }
    const { container } = render(<Controlled />);
    const trigger = screen.getByRole("combobox", { name: "Country" });
    expect(trigger.textContent).toContain("South Korea");
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(changes).toEqual(["SE"]);
    expect(trigger.textContent).toContain("Sweden");
    expect((container.querySelector('input[name="country"]') as HTMLInputElement).value).toBe("SE");
  });

  it("keeps a controlled value that the parent does not change", () => {
    const onChange = vi.fn();
    const { container } = render(<Select aria-label="Country" name="country" options={COUNTRIES} value="KR" onChange={onChange} />);
    const trigger = screen.getByRole("combobox", { name: "Country" });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith("SE");
    expect(trigger.textContent).toContain("South Korea");
    expect((container.querySelector('input[name="country"]') as HTMLInputElement).value).toBe("KR");
  });

  it("does not open when disabled", () => {
    const { trigger } = setup({ disabled: true, name: "country" });
    expect((trigger as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.click(trigger);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect((document.querySelector('input[name="country"]') as HTMLInputElement).disabled).toBe(true);
  });

  it("takes its name from an external label and labels the listbox with it", () => {
    render(
      <>
        <label htmlFor="country-select">Country</label>
        <Select id="country-select" options={COUNTRIES} defaultValue="" />
      </>,
    );
    const trigger = screen.getByRole("combobox", { name: "Country" });
    expect(trigger.id).toBe("country-select");
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    expect(screen.getByRole("listbox", { name: "Country" })).toBeTruthy();
  });

  it("opens from its label, and a press on the label while open closes it", () => {
    render(
      <>
        <label htmlFor="country-select">Country</label>
        <Select id="country-select" options={COUNTRIES} defaultValue="" />
      </>,
    );
    const label = screen.getByText("Country");
    const trigger = screen.getByRole("combobox", { name: "Country" });
    press(label);
    expect(screen.getByRole("listbox")).toBeTruthy();
    expect(document.activeElement).toBe(trigger);
    press(label);
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("renders option groups with labelled group roles", () => {
    render(
      <Select
        aria-label="Timezone"
        defaultValue="Asia/Seoul"
        options={[
          { value: "auto", label: "Auto" },
          { label: "Americas", options: [{ value: "America/New_York", label: "America/New York" }] },
          { label: "Asia", options: [{ value: "Asia/Seoul", label: "Asia/Seoul" }, { value: "Asia/Tokyo", label: "Asia/Tokyo" }] },
        ]}
      />,
    );
    const trigger = screen.getByRole("combobox", { name: "Timezone" });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    const asia = screen.getByRole("group", { name: "Asia" });
    expect(within(asia).getAllByRole("option").map((option) => option.textContent)).toEqual(["Asia/Seoul", "Asia/Tokyo"]);
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(trigger.textContent).toContain("Asia/Tokyo");
  });

  it("shows an inline prefix that is not part of the value text", () => {
    render(<Select aria-label="Usage display" prefix="Display" options={[{ value: "daily", label: "Daily" }]} />);
    const trigger = screen.getByRole("combobox", { name: "Usage display" });
    const prefix = within(trigger).getByText("Display");
    expect(prefix.getAttribute("aria-hidden")).toBe("true");
  });
});
