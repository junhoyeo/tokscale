"use client";

import { useId, useRef } from "react";
import { Select } from "@/components/ui/Select";

export interface FilterOption {
  value: string;
  label: string;
}

export interface DirectoryFilterClasses {
  filters: string;
  field: string;
  input: string;
  select: string;
  check: string;
  button: string;
}

/** Clean query string from the form: empty fields are dropped and "has" values are joined. */
function queryFromForm(form: HTMLFormElement): string {
  const params = new URLSearchParams();
  const has: string[] = [];
  for (const [key, value] of new FormData(form).entries()) {
    const text = String(value).trim();
    if (!text) continue;
    if (key === "has") has.push(text);
    else params.set(key, text);
  }
  if (has.length > 0) params.set("has", has.sort().join(","));
  if (params.get("sort") === "activity") params.delete("sort");
  return params.toString();
}

/**
 * Directory filters as a plain GET form. Without JavaScript it submits normally. With it, a
 * change submits at once and the URL drops empty fields, so the view stays short and shareable.
 * The page is server-rendered from the query string either way.
 */
export function DirectoryFilters({
  classes,
  action,
  q,
  selects,
  checks,
  hidden,
}: {
  classes: DirectoryFilterClasses;
  action: string;
  q: string;
  selects: Array<{ name: string; label: string; value: string; options: FilterOption[] }>;
  checks: Array<{ name: string; value: string; label: string; checked: boolean }>;
  hidden: Record<string, string>;
}) {
  const formRef = useRef<HTMLFormElement>(null);
  const idBase = useId();

  const go = () => {
    const form = formRef.current;
    if (!form) return;
    const query = queryFromForm(form);
    window.location.assign(query ? `${action}?${query}` : action);
  };

  return (
    <form
      ref={formRef}
      className={classes.filters}
      action={action}
      method="get"
      role="search"
      onSubmit={(event) => {
        event.preventDefault();
        go();
      }}
    >
      <label className={classes.field}>
        Search
        <input className={classes.input} type="search" name="q" defaultValue={q} placeholder="Name, alias or ticker" />
      </label>
      {selects.map((select) => {
        const selectId = `${idBase}${select.name}`;
        return (
          <div className={classes.field} key={select.name}>
            <label htmlFor={selectId} id={`${selectId}-label`}>
              {select.label}
            </label>
            <Select
              id={selectId}
              aria-labelledby={`${selectId}-label`}
              className={classes.select}
              name={select.name}
              defaultValue={select.value}
              options={select.options}
              onChange={go}
            />
          </div>
        );
      })}
      {checks.map((check) => (
        <label className={classes.check} key={`${check.name}-${check.value}`}>
          <input type="checkbox" name={check.name} value={check.value} defaultChecked={check.checked} onChange={go} />
          {check.label}
        </label>
      ))}
      {Object.entries(hidden).map(([name, value]) => (
        <input type="hidden" name={name} value={value} key={name} />
      ))}
      <button className={classes.button} type="submit">
        Apply
      </button>
    </form>
  );
}
