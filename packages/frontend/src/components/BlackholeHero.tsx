"use client";

import { useState } from "react";
import Image from "next/image";
import Link from "next/link";
import heroBg from "@/../public/assets/hero-bg.png";

export function BlackholeHero() {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText("npx tokscale");
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div 
      className="relative w-full max-w-7xl mx-auto mb-10 overflow-hidden h-[424px]"
      style={{
        borderRadius: "0px 0px 20px 20px",
        borderBottom: "1px solid rgba(105, 105, 105, 0.4)",
        borderLeft: "1px solid rgba(105, 105, 105, 0.4)",
        borderRight: "1px solid rgba(105, 105, 105, 0.4)",
      }}
    >
      <div className="absolute inset-0 z-0 bg-black">
        <Image
          src={heroBg}
          alt=""
          fill
          className="object-cover"
          priority
          quality={100}
          placeholder="blur"
        />
      </div>

      <div className="relative z-10 w-full h-full flex flex-col items-center pt-[53px] gap-[39px]">
        <div className="relative w-[173px] h-[36px] shrink-0">
          <Image
            src="/assets/hero-logo.svg"
            alt="Tokscale Logo"
            fill
            className="object-contain"
          />
        </div>

        <h1
          className="text-[48px] font-bold text-white text-center"
          style={{
            fontFamily: "Figtree, var(--font-geist-sans), sans-serif",
            lineHeight: "0.94em",
            letterSpacing: "-0.05em",
            textShadow: "0px 6px 12px 0px rgba(0, 30, 66, 0.35)",
          }}
        >
          The Kardashev Scale
          <br />
          for AI Devs
        </h1>

        <div
          className="flex items-center gap-[6px] p-[8px] rounded-xl border backdrop-blur-sm"
          style={{
            backgroundColor: "#141415",
            borderColor: "rgba(49, 56, 65, 0.4)",
          }}
        >
          <button
            onClick={handleCopy}
            className="flex items-center justify-center rounded-lg transition-all hover:opacity-90 active:scale-95 shrink-0"
            style={{
              backgroundColor: "#0073FF",
              height: "36px",
              width: "86px",
            }}
          >
            <span className="text-[15px] font-bold text-white leading-none tracking-tight">
              {copied ? "Copied" : "Copy"}
            </span>
          </button>
          
          <div 
            className="flex items-center relative overflow-hidden h-[36px] bg-[#1A1B1C] rounded-lg shrink-0 px-3"
            style={{ width: "190px" }}
          >
            <div className="z-10 flex items-center">
              <span
                style={{
                  color: "#FFF",
                  fontFamily: "Inconsolata, monospace",
                  fontSize: "16px",
                  fontWeight: 500,
                  lineHeight: "94%",
                  letterSpacing: "-0.8px",
                }}
              >
                npx&nbsp;
              </span>
              <span
                style={{
                  background: "linear-gradient(90deg, #0CF 0%, #0073FF 100%)",
                  backgroundClip: "text",
                  WebkitBackgroundClip: "text",
                  WebkitTextFillColor: "transparent",
                  fontFamily: "Inconsolata, monospace",
                  fontSize: "16px",
                  fontWeight: 500,
                  lineHeight: "94%",
                  letterSpacing: "-0.8px",
                }}
              >
                tokscale
              </span>
            </div>
            <div 
              className="ml-[10px] shrink-0"
              style={{
                width: "25px",
                height: "36px",
                background: "linear-gradient(270deg, rgba(26, 27, 28, 0) 0%, rgba(1, 127, 255, 0.14) 50%, rgba(26, 27, 28, 0) 100%)"
              }}
            />
          </div>
        </div>

        <div className="flex flex-col items-center gap-[4px]">
          <div className="flex items-center gap-[6px]">
            <Image 
              src="/assets/github-icon.svg" 
              alt="GitHub Star" 
              width={24} 
              height={24}
              className="block"
            />
            <span
              className="text-[18px] font-bold text-white"
              style={{ fontFamily: "Figtree, var(--font-geist-sans), sans-serif" }}
            >
              Star me on GitHub!
            </span>
          </div>
          <Link
            href="https://github.com/junhoyeo/tokscale"
            target="_blank"
            rel="noopener noreferrer"
            className="text-[16px] font-semibold transition-colors hover:text-white"
            style={{
              color: "#696969",
              fontFamily: "Figtree, var(--font-geist-sans), sans-serif"
            }}
          >
            junhoyeo/tokscale
          </Link>
        </div>
      </div>
    </div>
  );
}
