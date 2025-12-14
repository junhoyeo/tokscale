"use client";

import React from "react";
import { StyledComponentsRegistry } from "./StyledComponentsRegistry";
import { PrimerProvider } from "./PrimerProvider";

/**
 * Root providers wrapper for the application
 * 
 * Combines all necessary providers:
 * - StyledComponentsRegistry: SSR support for styled-components
 * - PrimerProvider: Primer design system theming
 */
export function Providers({ children }: { children: React.ReactNode }) {
  return (
    <StyledComponentsRegistry>
      <PrimerProvider>
        {children}
      </PrimerProvider>
    </StyledComponentsRegistry>
  );
}
