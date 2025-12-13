declare module "@opentui/core" {
  export interface CliRendererConfig {
    exitOnCtrlC?: boolean;
    targetFps?: number;
    backgroundColor?: string;
    useAlternateScreen?: boolean;
    useMouse?: boolean;
    gatherStats?: boolean;
    useKittyKeyboard?: Record<string, unknown> | null;
  }

  export interface CliRenderer {
    root: {
      add: (renderable: unknown) => void;
    };
    start: () => void;
    stop: () => void;
    console: {
      show: () => void;
    };
  }

  export function createCliRenderer(config?: CliRendererConfig): Promise<CliRenderer>;
  
  export interface KeyEvent {
    name: string;
    eventType: "press" | "release";
    repeated?: boolean;
    ctrl?: boolean;
    meta?: boolean;
    shift?: boolean;
    super?: boolean;
  }
}

declare module "@opentui/solid" {
  import type { Accessor, JSX as SolidJSX } from "solid-js";
  import type { CliRendererConfig, CliRenderer, KeyEvent } from "@opentui/core";

  export function render(
    node: () => SolidJSX.Element,
    config?: CliRendererConfig
  ): Promise<void>;

  export function useKeyboard(
    handler: (key: KeyEvent) => void,
    options?: { release?: boolean }
  ): void;

  export function useTerminalDimensions(): Accessor<{
    width: number;
    height: number;
  }>;

  export function useRenderer(): CliRenderer;

  export function useOnResize(callback: (width: number, height: number) => void): void;
}

declare namespace JSX {
  interface MouseEvent {
    x: number;
    y: number;
    button: number;
    type: "down" | "up" | "move" | "drag" | "scroll";
  }

  interface MouseEventHandlers {
    onMouse?: (event: MouseEvent) => void;
    onMouseDown?: (event: MouseEvent) => void;
    onMouseUp?: (event: MouseEvent) => void;
    onMouseMove?: (event: MouseEvent) => void;
    onMouseDrag?: (event: MouseEvent) => void;
    onMouseOver?: (event: MouseEvent) => void;
    onMouseOut?: (event: MouseEvent) => void;
    onMouseScroll?: (event: MouseEvent) => void;
  }

  interface IntrinsicElements {
    box: {
      flexDirection?: "row" | "column";
      flexGrow?: number;
      flexShrink?: number;
      flexWrap?: "wrap" | "nowrap";
      justifyContent?: "flex-start" | "flex-end" | "center" | "space-between" | "space-around" | "space-evenly";
      alignItems?: "flex-start" | "flex-end" | "center" | "stretch" | "baseline";
      alignSelf?: "auto" | "flex-start" | "flex-end" | "center" | "stretch" | "baseline";
      gap?: number;
      width?: number | string;
      height?: number | string;
      minWidth?: number | string;
      minHeight?: number | string;
      maxWidth?: number | string;
      maxHeight?: number | string;
      padding?: number;
      paddingX?: number;
      paddingY?: number;
      paddingTop?: number;
      paddingRight?: number;
      paddingBottom?: number;
      paddingLeft?: number;
      margin?: number;
      marginTop?: number;
      marginRight?: number;
      marginBottom?: number;
      marginLeft?: number;
      position?: "relative" | "absolute";
      top?: number;
      right?: number;
      bottom?: number;
      left?: number;
      backgroundColor?: string;
      borderStyle?: "single" | "double" | "round" | "bold" | "singleDouble" | "doubleSingle" | "classic";
      borderColor?: string;
      borderTop?: boolean;
      borderRight?: boolean;
      borderBottom?: boolean;
      borderLeft?: boolean;
      overflow?: "visible" | "hidden" | "scroll";
      children?: unknown;
    } & MouseEventHandlers;
    text: {
      fg?: string;
      bg?: string;
      backgroundColor?: string;
      bold?: boolean;
      dim?: boolean;
      italic?: boolean;
      underline?: boolean;
      strikethrough?: boolean;
      inverse?: boolean;
      wrap?: "wrap" | "truncate" | "truncate-start" | "truncate-middle" | "truncate-end";
      children?: unknown;
    } & MouseEventHandlers;
  }
}
