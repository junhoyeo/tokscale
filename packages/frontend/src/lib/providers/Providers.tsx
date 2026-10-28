"use client";

import React from "react";
import { createGlobalStyle } from "styled-components";
import reset from "styled-reset";
import { StyledComponentsRegistry } from "./StyledComponentsRegistry";


const GlobalStyle = createGlobalStyle`
  ${reset}
  
  *, *::before, *::after {
    box-sizing: border-box;
  }
  
  html {
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }
  
  body {
    font-family: var(--font-figtree), "Figtree", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    line-height: 1.5;
  }
  
  code, pre, kbd, samp {
    font-family: var(--font-mono), "JetBrains Mono", "Inconsolata", monospace !important;
  }
  
  img, picture, video, canvas, svg {
    display: block;
    max-width: 100%;
  }
  
  input, button, textarea, select {
    font: inherit;
  }
  
  p, h1, h2, h3, h4, h5, h6 {
    overflow-wrap: break-word;
  }
  
  a {
    color: inherit;
    text-decoration: none;
  }
  
  button {
    background: none;
    border: none;
    cursor: pointer;
  }

  /* One focus treatment on the site: a 2px ring in the focus blue. Components
     may place it closer to their own shape, never remove it. */
  :focus-visible {
    outline: 2px solid var(--v2-focus, #75b6ff);
    outline-offset: 2px;
  }

  ::selection {
    background: var(--v2-accent, #0073ff);
    color: #ffffff;
  }
`;

export function Providers({ children }: { children: React.ReactNode }) {
  return (
    <StyledComponentsRegistry>
      <GlobalStyle />
      {children}
    </StyledComponentsRegistry>
  );
}

