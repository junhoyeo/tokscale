"use client";

import { useState, useEffect } from "react";
import { useRouter } from "next/navigation";
import { Avatar, Button, Flash } from "@primer/react";
import { KeyIcon } from "@primer/octicons-react";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";

interface User {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  email: string | null;
}

interface ApiToken {
  id: string;
  name: string;
  createdAt: string;
  lastUsedAt: string | null;
}

export default function SettingsClient() {
  const router = useRouter();
  const [user, setUser] = useState<User | null>(null);
  const [tokens, setTokens] = useState<ApiToken[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    fetch("/api/auth/session")
      .then((res) => res.json())
      .then((data) => {
        if (!data.user) {
          router.push("/api/auth/github?returnTo=/settings");
          return;
        }
        setUser(data.user);
        setIsLoading(false);
      })
      .catch(() => {
        router.push("/");
      });

    fetch("/api/settings/tokens")
      .then((res) => res.json())
      .then((data) => {
        if (data.tokens) {
          setTokens(data.tokens);
        }
      })
      .catch(() => {});
  }, [router]);

  const handleRevokeToken = async (tokenId: string) => {
    if (!confirm("Are you sure you want to revoke this token?")) return;

    try {
      const response = await fetch(`/api/settings/tokens/${tokenId}`, {
        method: "DELETE",
      });

      if (response.ok) {
        setTokens(tokens.filter((t) => t.id !== tokenId));
      }
    } catch {
      alert("Failed to revoke token");
    }
  };

  if (isLoading) {
    return (
      <div className="min-h-screen flex flex-col" style={{ backgroundColor: "var(--color-bg-default)" }}>
        <Navigation />
        <main className="flex-1 flex items-center justify-center">
          <div style={{ color: "var(--color-fg-muted)" }}>Loading...</div>
        </main>
        <Footer />
      </div>
    );
  }

  if (!user) {
    return null;
  }

  return (
    <div className="min-h-screen flex flex-col" style={{ backgroundColor: "var(--color-bg-default)" }}>
      <Navigation />

      <main className="flex-1 max-w-3xl mx-auto px-6 py-10 w-full">
        <h1 className="text-3xl font-bold mb-8" style={{ color: "var(--color-fg-default)" }}>
          Settings
        </h1>

        <section
          className="rounded-2xl border p-6 mb-6"
          style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
        >
          <h2 className="text-lg font-semibold mb-4" style={{ color: "var(--color-fg-default)" }}>
            Profile
          </h2>
          <div className="flex items-center gap-4">
            <Avatar
              src={user.avatarUrl || `https://github.com/${user.username}.png`}
              alt={user.username}
              size={64}
              square
            />
            <div>
              <p className="font-medium" style={{ color: "var(--color-fg-default)" }}>
                {user.displayName || user.username}
              </p>
              <p className="text-sm" style={{ color: "var(--color-fg-muted)" }}>
                @{user.username}
              </p>
              {user.email && (
                <p className="text-sm" style={{ color: "var(--color-fg-muted)" }}>
                  {user.email}
                </p>
              )}
            </div>
          </div>
          <Flash variant="default" className="mt-4">
            Profile information is synced from GitHub and cannot be edited here.
          </Flash>
        </section>

        <section
          className="rounded-2xl border p-6 mb-6"
          style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
        >
          <h2 className="text-lg font-semibold mb-4" style={{ color: "var(--color-fg-default)" }}>
            API Tokens
          </h2>
          <p className="text-sm mb-4" style={{ color: "var(--color-fg-muted)" }}>
            Tokens are created when you run{" "}
            <code
              className="px-1 py-0.5 rounded text-xs"
              style={{ backgroundColor: "var(--color-bg-subtle)" }}
            >
              tokscale login
            </code>{" "}
            from the CLI.
          </p>

          {tokens.length === 0 ? (
            <div className="py-8 text-center" style={{ color: "var(--color-fg-muted)" }}>
              <KeyIcon size={32} className="mx-auto mb-3 opacity-50" />
              <p>No API tokens yet.</p>
              <p className="text-sm mt-2">
                Run{" "}
                <code
                  className="px-1 py-0.5 rounded text-xs"
                  style={{ backgroundColor: "var(--color-bg-subtle)" }}
                >
                  tokscale login
                </code>{" "}
                to create one.
              </p>
            </div>
          ) : (
            <div className="space-y-3">
              {tokens.map((token) => (
                <div
                  key={token.id}
                  className="flex items-center justify-between p-4 rounded-xl"
                  style={{ backgroundColor: "var(--color-bg-elevated)" }}
                >
                  <div className="flex items-center gap-3">
                    <KeyIcon size={20} className="text-neutral-500" />
                    <div>
                      <p className="font-medium" style={{ color: "var(--color-fg-default)" }}>
                        {token.name}
                      </p>
                      <p className="text-sm" style={{ color: "var(--color-fg-muted)" }}>
                        Created {new Date(token.createdAt).toLocaleDateString()}
                        {token.lastUsedAt && (
                          <> - Last used {new Date(token.lastUsedAt).toLocaleDateString()}</>
                        )}
                      </p>
                    </div>
                  </div>
                  <Button
                    variant="danger"
                    size="small"
                    onClick={() => handleRevokeToken(token.id)}
                  >
                    Revoke
                  </Button>
                </div>
              ))}
            </div>
          )}
        </section>


      </main>

      <Footer />
    </div>
  );
}
