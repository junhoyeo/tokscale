"use client";

/**
 * The last resort, when the root layout itself fails: this file must render its
 * own <html> and <body>, so it keeps the canvas and the type but not the shell.
 */
export default function GlobalError({ error, reset }: { error: Error & { digest?: string }; reset: () => void }) {
  return (
    <html lang="en">
      <body
        style={{
          margin: 0,
          minHeight: "100dvh",
          background: "#000",
          color: "#f4f7fb",
          fontFamily: "Figtree, system-ui, -apple-system, sans-serif",
        }}
      >
        <main style={{ maxWidth: 640, margin: "0 auto", padding: "96px 32px" }}>
          <h1 style={{ margin: 0, fontSize: "clamp(2.25rem, 4.5vw, 4rem)", fontWeight: 500, letterSpacing: "-0.02em", lineHeight: 1.05 }}>
            Tokscale could not start this page
          </h1>
          <p style={{ margin: "24px 0 0", color: "#a3adbd", fontSize: 17, lineHeight: 1.6 }}>
            Something failed before the page could load. Reloading usually fixes it.
          </p>
          <p style={{ margin: "32px 0 0" }}>
            <button
              type="button"
              onClick={reset}
              style={{
                minHeight: 44,
                padding: "0 18px",
                border: 0,
                borderRadius: 2,
                background: "#0066e6",
                color: "#fff",
                font: "inherit",
                fontSize: 16,
                cursor: "pointer",
              }}
            >
              Reload the page
            </button>
          </p>
          {error.digest ? (
            <p style={{ margin: "24px 0 0", color: "#848d9c", fontSize: 13 }}>
              Reference: {error.digest}
            </p>
          ) : null}
        </main>
      </body>
    </html>
  );
}
