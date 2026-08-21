import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Produce a self-contained output directory for Docker / Node deployments.
  // Vercel ignores this and uses its own file-tracing pipeline, so the
  // production deploy is unaffected. Local `next build` will emit an
  // additional .next/standalone directory alongside the normal output.
  output: "standalone",

  // Enable styled-components SSR
  compiler: {
    styledComponents: true,
  },

  // Allow images from GitHub
  images: {
    remotePatterns: [
      {
        protocol: "https",
        hostname: "avatars.githubusercontent.com",
        pathname: "/**",
      },
      {
        protocol: "https",
        hostname: "github.com",
        pathname: "/**",
      },
    ],
  },

  // Security headers for production
  headers: async () => [
    {
      source: "/:path*",
      headers: [
        {
          key: "X-DNS-Prefetch-Control",
          value: "on",
        },
        {
          key: "X-Frame-Options",
          value: "SAMEORIGIN",
        },
        {
          key: "X-Content-Type-Options",
          value: "nosniff",
        },
        {
          key: "Referrer-Policy",
          value: "strict-origin-when-cross-origin",
        },
      ],
    },
  ],

  // Experimental features
  experimental: {
    // Enable server actions
    serverActions: {
      bodySizeLimit: "2mb",
    },
  },
};

export default nextConfig;
