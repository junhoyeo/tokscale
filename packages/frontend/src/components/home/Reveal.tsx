"use client";

import { useLayoutEffect, useRef, type ElementType, type ReactNode } from "react";
import { createGlobalStyle } from "styled-components";

/**
 * The entrance layer: the parts of a section arrive in reading order, rising
 * 20px with opacity, once, staggered by a `delay` in ms.
 *
 * The hidden starting state exists only once a component has armed the element
 * on the client (`data-reveal="armed"`), so server-rendered content and
 * visitors with reduced motion always see the finished page. One observer and
 * one passive scroll fallback serve every element on the page: anything that
 * has been scrolled to — or scrolled past, as a fast jump can do — is
 * revealed, so no content can ever stay hidden.
 */

const pending = new Map<HTMLElement, () => void>();
let observer: IntersectionObserver | null = null;
let listening = false;

function connect() {
  if (listening || typeof window === "undefined") return;
  listening = true;

  const reveal = (el: HTMLElement) => {
    const done = pending.get(el);
    if (!done) return;
    done();
    pending.delete(el);
    observer?.unobserve(el);
    if (!pending.size) teardown();
  };

  if ("IntersectionObserver" in window && !window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) reveal(entry.target as HTMLElement);
        }
      },
      { threshold: 0, rootMargin: "0px 0px -10% 0px" },
    );
    for (const el of pending.keys()) observer.observe(el);
  }

  // Fallback: a jump (anchor, Page Down, restored scroll) can pass an element
  // without the observer ever seeing it in view. Anything at or above the
  // viewport is revealed.
  let frame = 0;
  const onScroll = () => {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      for (const el of [...pending.keys()]) {
        const r = el.getBoundingClientRect();
        if (r.bottom <= 0 || r.top < window.innerHeight * 0.95) reveal(el);
      }
    });
  };
  window.addEventListener("scroll", onScroll, { passive: true });
  window.addEventListener("resize", onScroll);
  function teardown() {
    window.removeEventListener("scroll", onScroll);
    window.removeEventListener("resize", onScroll);
    observer?.disconnect();
    observer = null;
    if (frame) cancelAnimationFrame(frame);
    listening = false;
  }
}

export function Reveal({
  children,
  delay = 0,
  as: Tag = "div",
  className,
  id,
}: {
  children: ReactNode;
  delay?: number;
  as?: ElementType;
  className?: string;
  id?: string;
}) {
  const ref = useRef<HTMLElement | null>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.dataset.reveal = "armed";

    if (
      !("IntersectionObserver" in window) ||
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      el.classList.add("is-in");
      return;
    }

    pending.set(el, () => el.classList.add("is-in"));
    connect();
    observer?.observe(el);
    return () => {
      pending.delete(el);
    };
  }, []);

  return (
    <Tag ref={ref} className={className} id={id} style={{ "--d": `${delay}ms` } as React.CSSProperties}>
      {children}
    </Tag>
  );
}

/** Mounted once per page. Easing and timing tokens for every move on it. */
export const MotionStyles = createGlobalStyle`
  :root {
    --ease-out: cubic-bezier(0.22, 1, 0.36, 1);
    --ease-expo: cubic-bezier(0.16, 1, 0.3, 1);
    --ease-in-out: cubic-bezier(0.76, 0, 0.24, 1);
    --ease-soft: cubic-bezier(0.33, 1, 0.68, 1);
  }

  [data-reveal="armed"] {
    opacity: 0;
    transform: translateY(20px);
  }

  @media (prefers-reduced-motion: no-preference) {
    [data-reveal="armed"] {
      transition:
        opacity 620ms var(--ease-out) var(--d, 0ms),
        transform 820ms var(--ease-out) var(--d, 0ms);
    }

    [data-reveal="armed"].is-in {
      opacity: 1;
      transform: none;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    [data-reveal="armed"] {
      opacity: 1;
      transform: none;
    }
  }
`;
