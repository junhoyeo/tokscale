'use client';

import { useState, useEffect, useId, useMemo, useCallback, useRef } from 'react';
import { getSvgPath } from 'figma-squircle';

export interface UseSquircleBorderOptions {
  cornerRadius: number;
  borderWidth: number;
  cornerSmoothing?: number;
  enabled?: boolean;
  initialWidth?: number;
  initialHeight?: number;
}

export interface SquirclePaths {
  outer: string;
  inner: string;
  safeWidth: number;
  safeHeight: number;
}

export interface UseSquircleBorderResult<T extends HTMLElement = HTMLElement> {
  ref: React.RefCallback<T>;
  containerRef: React.RefObject<T | null>;
  dimensions: { width: number; height: number };
  paths: SquirclePaths;
  clipId: string;
  maskId: string;
  outerClipId: string;
  innerClipId: string;
}

export function useSquircleBorder<T extends HTMLElement = HTMLElement>(
  options: UseSquircleBorderOptions
): UseSquircleBorderResult<T> {
  const {
    cornerRadius,
    borderWidth,
    cornerSmoothing = 1,
    enabled = true,
    initialWidth = 0,
    initialHeight = 0,
  } = options;

  const uniqueId = useId();
  const containerRef = useRef<T | null>(null);
  const [dimensions, setDimensions] = useState({
    width: initialWidth,
    height: initialHeight,
  });

  const outerClipId = `squircleOuterClip-${uniqueId}`;
  const innerClipId = `squircleInnerClip-${uniqueId}`;
  const maskId = `squircleBorderMask-${uniqueId}`;
  const clipId = outerClipId;

  const paths = useMemo((): SquirclePaths => {
    const { width, height } = dimensions;

    const safeWidth = Math.max(width, borderWidth * 2 + 1);
    const safeHeight = Math.max(height, borderWidth * 2 + 1);

    const outerPath = getSvgPath({
      width: safeWidth,
      height: safeHeight,
      cornerRadius,
      cornerSmoothing,
    });

    const innerPath = getSvgPath({
      width: safeWidth - borderWidth * 2,
      height: safeHeight - borderWidth * 2,
      cornerRadius: Math.max(0, cornerRadius - borderWidth),
      cornerSmoothing,
    });

    return {
      outer: outerPath,
      inner: innerPath,
      safeWidth,
      safeHeight,
    };
  }, [dimensions, cornerRadius, borderWidth, cornerSmoothing]);

  useEffect(() => {
    if (!enabled || !containerRef.current) return;

    const element = containerRef.current;

    const updateDimensions = () => {
      if (!element) return;
      const { width, height } = element.getBoundingClientRect();
      setDimensions({
        width: Math.round(width),
        height: Math.round(height),
      });
    };

    updateDimensions();

    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry) {
        const { width, height } = entry.contentRect;
        setDimensions({
          width: Math.round(width),
          height: Math.round(height),
        });
      }
    });

    observer.observe(element);

    return () => observer.disconnect();
  }, [enabled]);

  const ref = useCallback(
    (node: T | null) => {
      containerRef.current = node;

      if (node && enabled) {
        const { width, height } = node.getBoundingClientRect();
        if (width > 0 || height > 0) {
          setDimensions({
            width: Math.round(width),
            height: Math.round(height),
          });
        }
      }
    },
    [enabled]
  );

  return {
    ref,
    containerRef,
    dimensions,
    paths,
    clipId,
    maskId,
    outerClipId,
    innerClipId,
  };
}

export default useSquircleBorder;
