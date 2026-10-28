"use client";

import { useEffect } from "react";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { PageMessage } from "@/components/layout/PageMessage";

export default function Error({ error, reset }: { error: Error & { digest?: string }; reset: () => void }) {
  useEffect(() => {
    console.error(error);
  }, [error]);

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <PageMessage
          title="This page could not be loaded"
          primary={{ label: "Try again", onClick: reset }}
          secondary={{ label: "Back to the home page", href: "/" }}
        >
          The server did not return the data this page needs. Trying again often works; if it keeps failing, the
          home page and the leaderboard are unaffected.
        </PageMessage>
      </main>
      <PublicFooter />
    </div>
  );
}
