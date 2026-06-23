"use client";

import React, { useState } from "react";
import { useServerInsertedHTML } from "next/navigation";
import { ServerStyleSheet, StyleSheetManager } from "styled-components";

/**
 * Styled-components SSR Registry for Next.js App Router
 * 
 * This component ensures that styled-components styles are collected
 * during server-side rendering and properly inserted into the HTML.
 * Required for @primer/react components to work correctly with SSR.
 */
export function StyledComponentsRegistry({
  children,
}: {
  children: React.ReactNode;
}) {
  // Only create stylesheet once with lazy initial state
  // This prevents hydration mismatches
  const [styledComponentsStyleSheet] = useState(() => new ServerStyleSheet());

  useServerInsertedHTML(() => {
    const styles = styledComponentsStyleSheet.getStyleElement();
    styledComponentsStyleSheet.instance.clearTag();
    return <>{styles}</>;
  });

  // On the client, just render children directly
  if (typeof window !== "undefined") {
    return <>{children}</>;
  }

  // On the server, wrap with StyleSheetManager
  return (
    <StyleSheetManager sheet={styledComponentsStyleSheet.instance}>
      {children}
    </StyleSheetManager>
  );
}
