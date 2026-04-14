"use client";

import { useState, useEffect, useMemo, useCallback, useRef, useId } from "react";
import { getSvgPath } from "figma-squircle";

export interface SquircleBorderDef {
  outerClipId: string;
  innerClipId: string;
  maskId: string;
  outerPath: string;
  innerPath: string;
  width: number;
  height: number;
  cornerRadius: number;
  borderWidth: number;
  bottomOnly: boolean;
}

export function useSquircleClip<T extends HTMLElement = HTMLElement>(
  cornerRadius: number,
  cornerSmoothing: number = 0.6,
  bottomOnly: boolean = false,
  borderWidth: number = 0
) {
  const ref = useRef<T | null>(null);
  const clipId = useId().replace(/:/g, "");
  const baseId = `squircle-clip-${clipId}`;
  const [dimensions, setDimensions] = useState({ width: 0, height: 0 });

  const clipResult = useMemo(() => {
    const { width, height } = dimensions;
    if (width === 0 || height === 0)
      return { clipPath: "", svgDef: null, borderDef: null };

    let borderDef: SquircleBorderDef | null = null;

    if (bottomOnly) {
      const effectiveHeight = height + cornerRadius;
      const outerPath = getSvgPath({
        width,
        height: effectiveHeight,
        cornerRadius,
        cornerSmoothing,
      });

      if (borderWidth > 0) {
        const innerPath = getSvgPath({
          width: width - borderWidth * 2,
          height: effectiveHeight - borderWidth * 2,
          cornerRadius: Math.max(0, cornerRadius - borderWidth),
          cornerSmoothing,
        });
        borderDef = {
          outerClipId: `${baseId}-border-outer`,
          innerClipId: `${baseId}-border-inner`,
          maskId: `${baseId}-border-mask`,
          outerPath,
          innerPath,
          width,
          height,
          cornerRadius,
          borderWidth,
          bottomOnly: true,
        };
      }

      return {
        clipPath: `url(#${baseId})`,
        svgDef: { id: baseId, path: outerPath, width, effectiveHeight, cornerRadius },
        borderDef,
      };
    }

    const outerPath = getSvgPath({ width, height, cornerRadius, cornerSmoothing });

    if (borderWidth > 0) {
      const innerPath = getSvgPath({
        width: width - borderWidth * 2,
        height: height - borderWidth * 2,
        cornerRadius: Math.max(0, cornerRadius - borderWidth),
        cornerSmoothing,
      });
      borderDef = {
        outerClipId: `${baseId}-border-outer`,
        innerClipId: `${baseId}-border-inner`,
        maskId: `${baseId}-border-mask`,
        outerPath,
        innerPath,
        width,
        height,
        cornerRadius,
        borderWidth,
        bottomOnly: false,
      };
    }

    return { clipPath: `path('${outerPath}')`, svgDef: null, borderDef };
  }, [baseId, borderWidth, bottomOnly, cornerRadius, cornerSmoothing, dimensions]);

  useEffect(() => {
    if (!ref.current) return;
    const el = ref.current;

    const update = () => {
      const { width, height } = el.getBoundingClientRect();
      setDimensions({ width: Math.round(width), height: Math.round(height) });
    };

    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const setElementRef = useCallback((node: T | null) => {
    ref.current = node;
    if (node) {
      const { width, height } = node.getBoundingClientRect();
      if (width > 0 || height > 0) {
        setDimensions({ width: Math.round(width), height: Math.round(height) });
      }
    }
  }, []);

  return {
    setElementRef,
    clipPath: clipResult.clipPath,
    svgDef: clipResult.svgDef,
    borderDef: clipResult.borderDef,
  };
}
