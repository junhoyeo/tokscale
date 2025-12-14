/**
 * Type declarations for obelisk.js
 * Based on https://github.com/nosir/obelisk.js
 * 
 * obelisk.js is an isometric pixel art library for JavaScript
 */

declare module "obelisk.js" {
  // =====================================
  // Geometry Classes
  // =====================================

  /**
   * 2D Point for canvas positioning
   */
  export class Point {
    x: number;
    y: number;
    constructor(x?: number, y?: number);
    toString(): string;
  }

  /**
   * 3D Point for isometric positioning
   */
  export class Point3D {
    x: number;
    y: number;
    z: number;
    constructor(x?: number, y?: number, z?: number);
    toGlobalCoordinates(offset?: Point): Point;
    toString(): string;
  }

  /**
   * Transformation matrix
   */
  export class Matrix {
    a: number;
    b: number;
    c: number;
    d: number;
    tx: number;
    ty: number;
    constructor(a?: number, b?: number, c?: number, d?: number, tx?: number, ty?: number);
  }

  // =====================================
  // Dimension Classes
  // =====================================

  /**
   * Cube dimension (3D box)
   */
  export class CubeDimension {
    xAxis: number;
    yAxis: number;
    zAxis: number;
    constructor(xAxis?: number, yAxis?: number, zAxis?: number);
    toString(): string;
  }

  /**
   * Brick dimension (flat 2D surface)
   */
  export class BrickDimension {
    xAxis: number;
    yAxis: number;
    constructor(xAxis?: number, yAxis?: number);
    toString(): string;
  }

  /**
   * Pyramid dimension
   */
  export class PyramidDimension {
    xAxis: number;
    yAxis: number;
    tall: boolean;
    constructor(xAxis?: number, tall?: boolean);
    toString(): string;
  }

  /**
   * Side X dimension (wall along X axis)
   */
  export class SideXDimension {
    xAxis: number;
    zAxis: number;
    constructor(xAxis?: number, zAxis?: number);
    toString(): string;
  }

  /**
   * Side Y dimension (wall along Y axis)
   */
  export class SideYDimension {
    yAxis: number;
    zAxis: number;
    constructor(yAxis?: number, zAxis?: number);
    toString(): string;
  }

  /**
   * Slope dimension
   */
  export class SlopeDimension {
    xAxis: number;
    yAxis: number;
    constructor(xAxis?: number, yAxis?: number);
    toString(): string;
  }

  /**
   * Line X dimension
   */
  export class LineXDimension {
    xAxis: number;
    constructor(xAxis?: number);
    toString(): string;
  }

  /**
   * Line Y dimension
   */
  export class LineYDimension {
    yAxis: number;
    constructor(yAxis?: number);
    toString(): string;
  }

  /**
   * Line Z dimension
   */
  export class LineZDimension {
    zAxis: number;
    constructor(zAxis?: number);
    toString(): string;
  }

  // =====================================
  // Color Classes
  // =====================================

  /**
   * Cube color with automatic shading
   */
  export class CubeColor {
    border: number;
    borderHighlight: number;
    left: number;
    right: number;
    horizontal: number;

    constructor(
      border?: number,
      borderHighlight?: number,
      left?: number,
      right?: number,
      horizontal?: number
    );

    /**
     * Create color from a single horizontal (top) color
     * Automatically generates shaded left/right colors
     */
    getByHorizontalColor(horizontalColor: number): CubeColor;

    toString(): string;
  }

  /**
   * Side color for walls
   */
  export class SideColor {
    border: number;
    inner: number;

    constructor(border?: number, inner?: number);
    getByInnerColor(innerColor: number): SideColor;
    toString(): string;
  }

  /**
   * Pyramid color
   */
  export class PyramidColor {
    border: number;
    borderHighlight: number;
    left: number;
    right: number;

    constructor(border?: number, borderHighlight?: number, left?: number, right?: number);
    getByRightColor(rightColor: number): PyramidColor;
    toString(): string;
  }

  /**
   * Line color
   */
  export class LineColor {
    border: number;
    constructor(border?: number);
    toString(): string;
  }

  /**
   * Slope color
   */
  export class SlopeColor {
    border: number;
    borderHighlight: number;
    left: number;
    right: number;
    leftSlope: number;
    rightSlope: number;

    constructor(
      border?: number,
      borderHighlight?: number,
      left?: number,
      right?: number,
      leftSlope?: number,
      rightSlope?: number
    );

    getByHorizontalColor(horizontalColor: number): SlopeColor;
    toString(): string;
  }

  // =====================================
  // Primitive Classes
  // =====================================

  /**
   * Base interface for all primitives
   */
  interface AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;
  }

  /**
   * 3D Cube primitive
   */
  export class Cube implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;

    constructor(
      dimension?: CubeDimension,
      color?: CubeColor,
      border?: boolean,
      useDefaultCanvas?: boolean
    );

    toString(): string;
  }

  /**
   * Flat brick/floor tile primitive
   */
  export class Brick implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;

    constructor(
      dimension?: BrickDimension,
      color?: SideColor,
      border?: boolean,
      useDefaultCanvas?: boolean
    );

    toString(): string;
  }

  /**
   * Pyramid primitive
   */
  export class Pyramid implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;

    constructor(
      dimension?: PyramidDimension,
      color?: PyramidColor,
      border?: boolean,
      useDefaultCanvas?: boolean
    );

    toString(): string;
  }

  /**
   * Side X wall primitive
   */
  export class SideX implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;

    constructor(
      dimension?: SideXDimension,
      color?: SideColor,
      border?: boolean,
      useDefaultCanvas?: boolean
    );

    toString(): string;
  }

  /**
   * Side Y wall primitive
   */
  export class SideY implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;

    constructor(
      dimension?: SideYDimension,
      color?: SideColor,
      border?: boolean,
      useDefaultCanvas?: boolean
    );

    toString(): string;
  }

  /**
   * Line X primitive
   */
  export class LineX implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;

    constructor(dimension?: LineXDimension, color?: LineColor, useDefaultCanvas?: boolean);

    toString(): string;
  }

  /**
   * Line Y primitive
   */
  export class LineY implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;

    constructor(dimension?: LineYDimension, color?: LineColor, useDefaultCanvas?: boolean);

    toString(): string;
  }

  /**
   * Line Z primitive
   */
  export class LineZ implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;

    constructor(dimension?: LineZDimension, color?: LineColor, useDefaultCanvas?: boolean);

    toString(): string;
  }

  // Slope primitives
  export class SlopeEast implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;
    constructor(
      dimension?: SlopeDimension,
      color?: SlopeColor,
      border?: boolean,
      useDefaultCanvas?: boolean
    );
  }

  export class SlopeNorth implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;
    constructor(
      dimension?: SlopeDimension,
      color?: SlopeColor,
      border?: boolean,
      useDefaultCanvas?: boolean
    );
  }

  export class SlopeSouth implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;
    constructor(
      dimension?: SlopeDimension,
      color?: SlopeColor,
      border?: boolean,
      useDefaultCanvas?: boolean
    );
  }

  export class SlopeWest implements AbstractPrimitive {
    canvas: HTMLCanvasElement;
    w: number;
    h: number;
    matrix: Matrix;
    constructor(
      dimension?: SlopeDimension,
      color?: SlopeColor,
      border?: boolean,
      useDefaultCanvas?: boolean
    );
  }

  // =====================================
  // Display Classes
  // =====================================

  /**
   * Pixel object for positioning primitives
   */
  export class PixelObject {
    x: number;
    y: number;
    canvas: HTMLCanvasElement;

    constructor(primitive: AbstractPrimitive, point3d?: Point3D);
    toString(): string;
  }

  /**
   * Main view class for rendering isometric graphics
   */
  export class PixelView {
    canvas: HTMLCanvasElement;
    context: CanvasRenderingContext2D;
    point: Point;

    constructor(canvas: HTMLCanvasElement, point?: Point);

    /**
     * Render a primitive at the given 3D position
     */
    renderObject(primitive: AbstractPrimitive, point3d?: Point3D): void;

    /**
     * Clear the canvas
     */
    clear(): void;

    toString(): string;
  }

  /**
   * Bitmap data for low-level pixel manipulation
   */
  export class BitmapData {
    canvas: HTMLCanvasElement;
    context: CanvasRenderingContext2D;
    imageData: ImageData;

    constructor(width: number, height: number, useDefaultCanvas?: boolean);

    setPixel(x: number, y: number, color: number): void;
    setPixelByIndex(index: number, color: number): void;
    checkPixelAvailable(x: number, y: number): boolean;
    floodFill(x: number, y: number, color: number): void;
    toString(): string;
  }

  // =====================================
  // Utility Classes
  // =====================================

  /**
   * Canvas manager for creating and managing canvases
   */
  export namespace CanvasManager {
    function getDefaultCanvas(): HTMLCanvasElement;
    function getNewCanvas(): HTMLCanvasElement;
  }

  /**
   * Canvas tool utilities
   */
  export namespace CanvasTool {
    function getPixel(imageData: ImageData, x: number, y: number): number;
  }

  /**
   * Color geometry utilities
   */
  export namespace ColorGeom {
    function get32(color: number): number;
    function applyBrightness(color: number, brightness: number, highlight?: boolean): number;
  }

  /**
   * Color pattern utilities
   */
  export namespace ColorPattern {
    const CYCLED: number;
    const RANDOM: number;
  }

  // Default export for the entire library
  const obelisk: {
    Point: typeof Point;
    Point3D: typeof Point3D;
    Matrix: typeof Matrix;
    CubeDimension: typeof CubeDimension;
    BrickDimension: typeof BrickDimension;
    PyramidDimension: typeof PyramidDimension;
    SideXDimension: typeof SideXDimension;
    SideYDimension: typeof SideYDimension;
    SlopeDimension: typeof SlopeDimension;
    LineXDimension: typeof LineXDimension;
    LineYDimension: typeof LineYDimension;
    LineZDimension: typeof LineZDimension;
    CubeColor: typeof CubeColor;
    SideColor: typeof SideColor;
    PyramidColor: typeof PyramidColor;
    LineColor: typeof LineColor;
    SlopeColor: typeof SlopeColor;
    Cube: typeof Cube;
    Brick: typeof Brick;
    Pyramid: typeof Pyramid;
    SideX: typeof SideX;
    SideY: typeof SideY;
    LineX: typeof LineX;
    LineY: typeof LineY;
    LineZ: typeof LineZ;
    SlopeEast: typeof SlopeEast;
    SlopeNorth: typeof SlopeNorth;
    SlopeSouth: typeof SlopeSouth;
    SlopeWest: typeof SlopeWest;
    PixelObject: typeof PixelObject;
    PixelView: typeof PixelView;
    BitmapData: typeof BitmapData;
    CanvasManager: typeof CanvasManager;
    CanvasTool: typeof CanvasTool;
    ColorGeom: typeof ColorGeom;
    ColorPattern: typeof ColorPattern;
  };

  export default obelisk;
}
