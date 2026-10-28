"use client";

import { useRef, useState } from "react";
import styled from "styled-components";
import { DitherField } from "dither-kit/DitherField";
import { DitherInput } from "dither-kit/DitherInput";
import { SecondaryButton } from "./RankingUI";
import { Select } from "@/components/ui/Select";
import { COMMON_TIMEZONE_GROUPS, isValidTimeZone } from "@/lib/timezone";

interface GroupReportingSettingsProps {
  slug: string;
  timeZone: string;
  weekStartsOn: number;
  onChanged: () => Promise<void>;
}

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const WEEKDAY_OPTIONS = WEEKDAYS.map((day, index) => ({ value: String(index), label: day }));

/* Settings row: the label column (title + one sentence) beside its panel,
 * split from the section above by a full-width rule. */
const Section = styled.section`
  display: grid;
  gap: 16px;
  margin-top: 32px;
  padding-top: 24px;
  border-top: 1px solid var(--service-border);

  @media (min-width: 900px) {
    grid-template-columns: 320px minmax(0, 1fr);
    gap: 40px;
  }
`;

const LabelColumn = styled.div`
  min-width: 0;
`;

const SectionTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 500;
`;

const SectionNote = styled.p`
  margin: 6px 0 0;
  max-width: 46ch;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
`;

const Panel = styled.div`
  display: grid;
  min-width: 0;
  align-content: start;
  gap: 16px;

  label {
    font-size: 0.875rem;
    font-weight: 500;
  }
`;

const Controls = styled.div`
  display: grid;
  gap: 16px;

  @media (min-width: 560px) {
    grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
  }
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px;
`;

const Notice = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
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

  const canSave = !pending
    && isValidTimeZone(selectedTimeZone)
    && (selectedTimeZone !== timeZone || selectedWeekStart !== weekStartsOn);

  return (
    <Section aria-label="Group reporting settings">
      <LabelColumn>
        <SectionTitle>Reporting calendar</SectionTitle>
        <SectionNote>
          The shared calendar for this group. Enter any IANA time zone; common zones are suggested. Submitted daily totals keep their original dates.
        </SectionNote>
      </LabelColumn>
      <Panel>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void saveCalendar();
          }}
        >
          <Controls>
            <DitherField label="Time zone">
              <DitherInput
                aria-label="Time zone"
                list="group-reporting-time-zones"
                value={selectedTimeZone}
                onChange={setSelectedTimeZone}
                disabled={pending}
                spellCheck={false}
                autoComplete="off"
              />
            </DitherField>
            <DitherField label="Week starts on">
              <Select
                aria-label="Week starts on"
                value={String(selectedWeekStart)}
                options={WEEKDAY_OPTIONS}
                onChange={(next) => setSelectedWeekStart(Number(next))}
                disabled={pending}
              />
            </DitherField>
          </Controls>
          <datalist id="group-reporting-time-zones">
            {COMMON_TIMEZONE_GROUPS.flatMap((group) => group.zones).map((zone) => (
              <option key={zone} value={zone} />
            ))}
          </datalist>
          <Actions>
            <SecondaryButton type="submit" disabled={!canSave}>Save calendar</SecondaryButton>
            {notice && <Notice role="status">{notice}</Notice>}
          </Actions>
        </form>
        {error && <Notice role="alert">{error}</Notice>}
      </Panel>
    </Section>
  );
}
