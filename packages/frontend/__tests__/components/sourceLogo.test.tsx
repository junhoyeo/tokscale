import { isValidElement, type ReactElement } from "react";
import { describe, expect, it } from "vitest";
import { SourceLogo } from "../../src/components/SourceLogo";
import { SOURCE_LOGOS } from "../../src/lib/constants";
import { SUPPORTED_CLIENT_TYPES } from "../../src/lib/types";

type SourceLogoElementProps = {
  alt?: string;
  children?: string;
  className?: string;
  src?: string;
};

function renderSourceLogo(sourceId: string): ReactElement<SourceLogoElementProps> {
  const element = SourceLogo({ sourceId, height: 16, className: "source-logo" });

  expect(isValidElement(element)).toBe(true);
  return element as ReactElement<SourceLogoElementProps>;
}

describe("SourceLogo", () => {
  it("uses the shared logo registry for every supported base client", () => {
    for (const client of SUPPORTED_CLIENT_TYPES) {
      const element = renderSourceLogo(client);

      expect(element.type).not.toBe("span");
      expect(element.props.src).toBe(SOURCE_LOGOS[client]);
      expect(element.props.alt).toBe(client);
      expect(element.props.className).toBe("source-logo");
    }
  });

  it("normalizes source ids before registry lookup", () => {
    const element = renderSourceLogo("CodeBuff");

    expect(element.type).not.toBe("span");
    expect(element.props.src).toBe(SOURCE_LOGOS.codebuff);
    expect(element.props.alt).toBe("CodeBuff");
  });

  it("falls back to text for unsupported variant ids", () => {
    const element = renderSourceLogo("cc-mirror/example");

    expect(element.type).toBe("span");
    expect(element.props.children).toBe("cc-mirror/example");
    expect(element.props.className).toBe("source-logo");
  });
});
