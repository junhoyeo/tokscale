"use client";

import { useRef, useState } from "react";
import styled from "styled-components";
import { SecondaryButton } from "./RankingUI";
import { COMMON_TIMEZONE_GROUPS, isValidTimeZone } from "@/lib/timezone";

interface GroupReportingSettingsProps {
  slug: string;
  timeZone: string;
  weekStartsOn: number;
  onChanged: () => Promise<void>;
}

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

const Panel = styled.section`
  display: grid;
  gap: 16px;
  margin-top: 24px;
  padding: 20px;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  background: var(--service-surface);
  h2, p { margin: 0; }
  h2 { font-size: 0.9375rem; font-weight: 600; }
  p { color: var(--service-text-muted); font-size: 0.875rem; line-height: 1.6; }
`;

const Controls = styled.div`
  display: flex;
  max-width: 100%;
  flex-wrap: wrap;
  align-items: end;
  gap: 12px;
  label { display: grid; max-width: 100%; gap: 6px; color: var(--service-text-muted); font-size: 0.8125rem; }
  select, input {
    min-height: 44px;
    max-width: 100%;
    padding: 8px;
    border: 1px solid var(--service-border-strong);
    border-radius: 8px;
    background: var(--service-surface);
    color: var(--service-text);
    font-size: 0.875rem;
    color-scheme: dark;
  }
  select:focus-visible, input:focus-visible { outline: 2px solid var(--service-focus); }
`;

export function GroupReportingSettings({ slug, timeZone, weekStartsOn, onChanged }: GroupReportingSettingsProps) {
  const [selectedTimeZone, setSelectedTimeZone] = useState(timeZone);
  const [selectedWeekStart, setSelectedWeekStart] = useState(weekStartsOn);
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  async function saveCalendar() {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPending(true);
    setError(null);
    setNotice(null);
    try {
      const response = await fetch(`/api/groups/${slug}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ timeZone: selectedTimeZone, weekStartsOn: selectedWeekStart }),
      });
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.error || "Could not update calendar");
      setNotice("Calendar saved.");
      try {
        await onChanged();
      } catch {
        setError("Calendar was saved, but the group could not be refreshed. Reload this page before making another change.");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not update calendar");
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  }

  return (
    <Panel aria-label="Group reporting settings">
      <h2>Reporting calendar</h2>
      <p>Choose the shared calendar for this group. Enter any IANA time zone; common zones are suggested. Submitted daily totals keep their original dates.</p>
      <form onSubmit={(event) => { event.preventDefault(); void saveCalendar(); }}>
        <Controls>
          <label>Time zone
            <input
              aria-label="Time zone"
              list="group-reporting-time-zones"
              value={selectedTimeZone}
              onChange={(event) => setSelectedTimeZone(event.target.value)}
              disabled={pending}
              spellCheck={false}
            />
            <datalist id="group-reporting-time-zones">
              {COMMON_TIMEZONE_GROUPS.flatMap((group) => group.zones).map((zone) => (
                <option key={zone} value={zone} />
              ))}
            </datalist>
          </label>
          <label>Week starts on
            <select aria-label="Week starts on" value={selectedWeekStart} onChange={(event) => setSelectedWeekStart(Number(event.target.value))} disabled={pending}>
              {WEEKDAYS.map((day, index) => <option key={day} value={index}>{day}</option>)}
            </select>
          </label>
          <SecondaryButton type="submit" disabled={pending || !isValidTimeZone(selectedTimeZone) || (selectedTimeZone === timeZone && selectedWeekStart === weekStartsOn)}>Save calendar</SecondaryButton>
        </Controls>
      </form>
      {notice && <p role="status">{notice}</p>}
      {error && <p role="alert">{error}</p>}
    </Panel>
  );
}
