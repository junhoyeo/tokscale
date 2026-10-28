"use client";

import { useEffect } from "react";

/**
 * One-way reveal for the landing page's section parts.
 *
 * The hidden state only exists once this runs (`html[data-reveal="on"]`), so
 * the page is complete without JavaScript, and a reader who asked for reduced
 * motion gets the finished state immediately. Each element is shown once as it
 * enters and is then unwatched: nothing ever goes back to zero opacity, so
 * scrolling up never makes the page empty again.
 */
export function Reveal() {
  useEffect(() => {
    const root = document.documentElement;
    const nodes = [...document.querySelectorAll("[data-reveal]")];
    if (nodes.length === 0) return;
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    // Everything already on screen is shown in the same frame that hides the rest,
    // so there is no flash and the first screen never waits for a scroll.
    const inView = (el: Element) => el.getBoundingClientRect().top < window.innerHeight * 0.92;
    root.dataset.reveal = "on";
    for (const el of nodes) if (inView(el)) el.setAttribute("data-reveal-in", "");

    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          entry.target.setAttribute("data-reveal-in", "");
          io.unobserve(entry.target);
        }
      },
      { rootMargin: "0px 0px -8% 0px", threshold: 0.06 }
    );
    for (const el of nodes) if (!el.hasAttribute("data-reveal-in")) io.observe(el);
    return () => {
      io.disconnect();
      delete root.dataset.reveal;
    };
  }, []);

  return null;
}
