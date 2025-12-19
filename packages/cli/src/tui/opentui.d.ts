declare module "@opentui/solid/preload" {
  const preload: void;
  export default preload;
}

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
    destroy: () => void;
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

// OpenTUI element types for JSX
// Using `any` for children due to JSX.Element type mismatch between solid-js and solid-js/h/jsx-runtime
type OpenTUIChildren = any;

interface OpenTUIMouseEvent {
  x: number;
  y: number;
  button: number;
  type: "down" | "up" | "move" | "drag" | "scroll";
}

interface OpenTUIMouseEventHandlers {
  onMouse?: (event: OpenTUIMouseEvent) => void;
  onMouseDown?: (event: OpenTUIMouseEvent) => void;
  onMouseUp?: (event: OpenTUIMouseEvent) => void;
  onMouseMove?: (event: OpenTUIMouseEvent) => void;
  onMouseDrag?: (event: OpenTUIMouseEvent) => void;
  onMouseOver?: (event: OpenTUIMouseEvent) => void;
  onMouseOut?: (event: OpenTUIMouseEvent) => void;
  onMouseScroll?: (event: OpenTUIMouseEvent) => void;
}

interface OpenTUIBoxProps extends OpenTUIMouseEventHandlers {
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
  children?: OpenTUIChildren;
}

interface OpenTUITextProps extends OpenTUIMouseEventHandlers {
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
  children?: OpenTUIChildren;
}

interface OpenTUISpanProps extends OpenTUIMouseEventHandlers {
  fg?: string;
  bg?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  strikethrough?: boolean;
  inverse?: boolean;
  children?: OpenTUIChildren;
}

// Module augmentation for solid-js/h/jsx-runtime
declare module "solid-js/h/jsx-runtime" {
  namespace JSX {
    interface IntrinsicElements {
      box: OpenTUIBoxProps;
      text: OpenTUITextProps;
      span: OpenTUISpanProps;
    }
  }
}
