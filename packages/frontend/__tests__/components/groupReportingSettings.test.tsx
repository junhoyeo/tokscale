import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { GroupReportingSettings } from "@/components/leaderboard/GroupReportingSettings";

describe("GroupReportingSettings", () => {
  it("accepts valid IANA zones beyond the common suggestions", () => {
    const markup = renderToStaticMarkup(
      <GroupReportingSettings
        slug="team"
        timeZone="Australia/Adelaide"
        weekStartsOn={1}
        onChanged={async () => {}}
      />,
    );

    expect(markup).toContain('input');
    expect(markup).toContain('list="group-reporting-time-zones"');
    expect(markup).toContain('value="Australia/Adelaide"');
    expect(markup).toContain('value="UTC"');
  });
});
