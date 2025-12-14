"use client";

import { useState, useEffect } from "react";
import { useRouter } from "next/navigation";
import { Avatar, Button, Flash } from "@primer/react";
import { TrashIcon, KeyIcon } from "@primer/octicons-react";
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

export default function SettingsPage() {
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
      <div className="min-h-screen flex flex-col" style={{ backgroundColor: "#141415" }}>
        <Navigation />
        <main className="flex-1 flex items-center justify-center">
          <div style={{ color: "#696969" }}>Loading...</div>
        </main>
        <Footer />
      </div>
    );
  }

  if (!user) {
    return null;
  }

  return (
    <div className="min-h-screen flex flex-col" style={{ backgroundColor: "#141415" }}>
      <Navigation />

      <main className="flex-1 max-w-3xl mx-auto px-6 py-10 w-full">
        <h1 className="text-3xl font-bold mb-8" style={{ color: "#FFFFFF" }}>
          Settings
        </h1>

        <section
          className="rounded-2xl border p-6 mb-6"
          style={{ backgroundColor: "#141415", borderColor: "#262627" }}
        >
          <h2 className="text-lg font-semibold mb-4" style={{ color: "#FFFFFF" }}>
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
              <p className="font-medium" style={{ color: "#FFFFFF" }}>
                {user.displayName || user.username}
              </p>
              <p className="text-sm" style={{ color: "#696969" }}>
                @{user.username}
              </p>
              {user.email && (
                <p className="text-sm" style={{ color: "#696969" }}>
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
          style={{ backgroundColor: "#141415", borderColor: "#262627" }}
        >
          <h2 className="text-lg font-semibold mb-4" style={{ color: "#FFFFFF" }}>
            API Tokens
          </h2>
          <p className="text-sm mb-4" style={{ color: "#696969" }}>
            Tokens are created when you run{" "}
            <code
              className="px-1 py-0.5 rounded text-xs"
              style={{ backgroundColor: "#262627" }}
            >
              tokscale login
            </code>{" "}
            from the CLI.
          </p>

          {tokens.length === 0 ? (
            <div className="py-8 text-center" style={{ color: "#696969" }}>
              <KeyIcon size={32} className="mx-auto mb-3 opacity-50" />
              <p>No API tokens yet.</p>
              <p className="text-sm mt-2">
                Run{" "}
                <code
                  className="px-1 py-0.5 rounded text-xs"
                  style={{ backgroundColor: "#262627" }}
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
                  style={{ backgroundColor: "#1F1F20" }}
                >
                  <div className="flex items-center gap-3">
                    <KeyIcon size={20} className="text-neutral-500" />
                    <div>
                      <p className="font-medium" style={{ color: "#FFFFFF" }}>
                        {token.name}
                      </p>
                      <p className="text-sm" style={{ color: "#696969" }}>
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

        <section
          className="rounded-2xl border p-6"
          style={{ backgroundColor: "#141415", borderColor: "#7F1D1D" }}
        >
          <h2 className="text-lg font-semibold mb-4" style={{ color: "#EF4444" }}>
            Danger Zone
          </h2>
          <p className="text-sm mb-4" style={{ color: "#696969" }}>
            Deleting your account will remove all your submissions and cannot be undone.
          </p>
          <Button
            variant="danger"
            leadingVisual={TrashIcon}
            onClick={() => {
              if (
                confirm(
                  "Are you sure you want to delete your account? This action cannot be undone."
                )
              ) {
                alert("Account deletion is not yet implemented.");
              }
            }}
          >
            Delete Account
          </Button>
        </section>
      </main>

      <Footer />
    </div>
  );
}
