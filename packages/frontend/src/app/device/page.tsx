"use client";

import { useState, useEffect } from "react";

interface User {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
}

export default function DevicePage() {
  const [user, setUser] = useState<User | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [code, setCode] = useState("");
  const [status, setStatus] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [error, setError] = useState("");

  useEffect(() => {
    fetch("/api/auth/session")
      .then((res) => res.json())
      .then((data) => {
        setUser(data.user);
        setIsLoading(false);
      })
      .catch(() => {
        setIsLoading(false);
      });
  }, []);

  const handleCodeChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    let value = e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, "");

    if (value.length > 4) {
      value = value.slice(0, 4) + "-" + value.slice(4, 8);
    }

    setCode(value);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setStatus("loading");
    setError("");

    try {
      const response = await fetch("/api/auth/device/authorize", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ userCode: code }),
      });

      if (!response.ok) {
        const data = await response.json();
        throw new Error(data.error || "Invalid code");
      }

      setStatus("success");
    } catch (err) {
      setStatus("error");
      setError(err instanceof Error ? err.message : "Something went wrong");
    }
  };

  if (isLoading) {
    return (
      <div
        className="min-h-screen flex items-center justify-center"
        style={{ backgroundColor: "#141415" }}
      >
        <div style={{ color: "#696969" }}>Loading...</div>
      </div>
    );
  }

  return (
    <div
      className="min-h-screen flex items-center justify-center px-4"
      style={{ backgroundColor: "#141415" }}
    >
      <div className="max-w-md w-full">
        <div
          className="rounded-2xl border p-8"
          style={{ backgroundColor: "#141415", borderColor: "#262627" }}
        >
          <div className="text-center mb-8">
            <div className="w-16 h-16 mx-auto mb-4 rounded-2xl flex items-center justify-center shadow-lg" style={{ background: "linear-gradient(to bottom right, #53d1f3, #3bc4e8)", boxShadow: "0 10px 15px -3px rgba(83, 209, 243, 0.25)" }}>
              <svg
                className="w-8 h-8 text-white"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
                />
              </svg>
            </div>
            <h1 className="text-2xl font-bold" style={{ color: "#FFFFFF" }}>
              Authorize CLI
            </h1>
            <p className="mt-2" style={{ color: "#696969" }}>
              Connect your terminal to Token Tracker
            </p>
          </div>

          {!user ? (
            <div className="text-center">
              <p className="mb-6" style={{ color: "#9CA3AF" }}>
                Sign in with GitHub to authorize the CLI.
              </p>
              <a
                href="/api/auth/github?returnTo=/device"
                className="inline-flex items-center justify-center gap-2 px-6 py-3 font-medium rounded-xl transition-opacity hover:opacity-80"
                style={{ backgroundColor: "#FFFFFF", color: "#141415" }}
              >
                <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
                </svg>
                Sign in with GitHub
              </a>
            </div>
          ) : status === "success" ? (
            <div className="text-center">
              <div
                className="w-16 h-16 mx-auto mb-4 rounded-full flex items-center justify-center"
                style={{ backgroundColor: "rgba(83, 209, 243, 0.1)" }}
              >
                <svg
                  className="w-8 h-8"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  style={{ color: "#53d1f3" }}
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M5 13l4 4L19 7"
                  />
                </svg>
              </div>
              <h2 className="text-xl font-semibold mb-2" style={{ color: "#FFFFFF" }}>
                Device Authorized!
              </h2>
              <p style={{ color: "#696969" }}>
                You can close this window and return to your terminal.
              </p>
            </div>
          ) : (
            <form onSubmit={handleSubmit}>
              <div className="mb-4">
                <p className="text-center mb-4" style={{ color: "#9CA3AF" }}>
                  Enter the code shown in your terminal:
                </p>
                <input
                  type="text"
                  value={code}
                  onChange={handleCodeChange}
                  placeholder="XXXX-XXXX"
                  maxLength={9}
                  className="w-full px-4 py-4 text-center text-2xl font-mono tracking-[0.3em] border rounded-xl focus:outline-none focus:ring-2 focus:border-transparent"
                  style={{
                    backgroundColor: "#1F1F20",
                    borderColor: "#262627",
                    color: "#FFFFFF",
                    // @ts-expect-error CSS custom property
                    "--tw-ring-color": "#53d1f3",
                  }}
                  autoFocus
                />
              </div>

              {error && (
                <p className="text-red-500 text-sm text-center mb-4">{error}</p>
              )}

              <button
                type="submit"
                disabled={code.length < 9 || status === "loading"}
                className="w-full px-6 py-3 text-white font-medium rounded-xl disabled:opacity-50 disabled:cursor-not-allowed transition-colors hover:opacity-90"
                style={{ backgroundColor: "#53d1f3" }}
              >
                {status === "loading" ? "Authorizing..." : "Authorize Device"}
              </button>

              <p className="text-center text-sm mt-4" style={{ color: "#696969" }}>
                Signed in as{" "}
                <span className="font-medium" style={{ color: "#9CA3AF" }}>
                  {user.username}
                </span>
              </p>
            </form>
          )}
        </div>
      </div>
    </div>
  );
}
