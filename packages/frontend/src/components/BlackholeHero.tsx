"use client";

import dynamic from "next/dynamic";
import { useEffect, useState, useCallback } from "react";
import { motion } from "framer-motion";

const BlackholeBackground = dynamic(
  () => import("@junhoyeo/blackhole").then((mod) => mod.BlackholeBackground),
  { ssr: false }
);

export function BlackholeHero() {
  const [key, setKey] = useState(0);
  const [isReady, setIsReady] = useState(false);

  useEffect(() => {
    let timeoutId: NodeJS.Timeout;
    const handleResize = () => {
      clearTimeout(timeoutId);
      timeoutId = setTimeout(() => {
        setIsReady(false);
        setKey((k) => k + 1);
      }, 150);
    };
    window.addEventListener("resize", handleResize);
    return () => {
      clearTimeout(timeoutId);
      window.removeEventListener("resize", handleResize);
    };
  }, []);

  const handleTexturesLoaded = useCallback(() => {
    setIsReady(true);
  }, []);

  return (
    <div
      className="relative w-full max-w-7xl mx-auto mb-10 overflow-hidden rounded-2xl"
      style={{
        height: "420px",
        backgroundColor: "#000",
      }}
    >
      <motion.div
        className="absolute"
        initial={{ opacity: 0 }}
        animate={{ opacity: isReady ? 1 : 0 }}
        transition={{ duration: 1, ease: "easeOut" }}
        style={{
          width: "max(100%, 900px)",
          height: "100%",
          left: "50%",
          top: 0,
          transform: "translateX(-50%)",
          zIndex: 0,
        }}
      >
        <BlackholeBackground
          key={key}
          quality="high"
          cameraDistance={10}
          fieldOfView={90}
          enableOrbit={true}
          showAccretionDisk={true}
          useDiskTexture={true}
          enableLorentzTransform={true}
          enableDopplerShift={true}
          enableBeaming={true}
          bloomStrength={0.5}
          bloomRadius={0.3}
          bloomThreshold={0.8}
          backgroundTextureUrl="/assets/milkyway.jpg"
          starTextureUrl="/assets/star_noise.png"
          diskTextureUrl="/assets/accretion_disk.png"
          onTexturesLoaded={handleTexturesLoaded}
        />
      </motion.div>

      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: isReady ? 1 : 0 }}
        transition={{ duration: 0.8, ease: "easeOut" }}
        className="absolute inset-0 flex flex-col items-center justify-center text-center pointer-events-none"
        style={{
          background: "radial-gradient(ellipse at center, transparent 0%, rgba(0,0,0,0.3) 100%)",
          zIndex: 2,
        }}
      >
        <motion.h1
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: isReady ? 1 : 0, y: isReady ? 0 : 20 }}
          transition={{ duration: 0.6, delay: 0.2, ease: "easeOut" }}
          className="text-3xl md:text-4xl lg:text-5xl font-bold mb-3"
          style={{
            color: "#FFFFFF",
            textShadow: "0 2px 20px rgba(0,0,0,0.8)",
            letterSpacing: "-0.03em",
          }}
        >
          The Kardashev Scale of AI Devs
        </motion.h1>
        <motion.p
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: isReady ? 1 : 0, y: isReady ? 0 : 20 }}
          transition={{ duration: 0.6, delay: 0.4, ease: "easeOut" }}
          className="text-lg md:text-xl max-w-md px-4"
          style={{ color: "rgba(255,255,255,0.8)", textShadow: "0 1px 10px rgba(0,0,0,0.8)" }}
        >
          Track your AI token usage across all platforms
        </motion.p>
      </motion.div>

      <div
        className="absolute bottom-0 left-0 right-0 pointer-events-none"
        style={{
          height: "100px",
          background: "linear-gradient(to bottom, rgba(20, 20, 21, 0) 0%, rgba(20, 20, 21, 1) 100%)",
          zIndex: 1,
        }}
      />
    </div>
  );
}
