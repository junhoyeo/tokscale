import Link from "next/link";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { PageMessage } from "@/components/layout/PageMessage";

export default function NotFound() {
  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <PageMessage title="That page does not exist" primary={{ label: "Back to the home page", href: "/" }}>
          The address may be mistyped, or the page may have moved. Everything Tokscale publishes starts on the home
          page and the <Link href="/leaderboard">leaderboard</Link>.
        </PageMessage>
      </main>
      <PublicFooter />
    </div>
  );
}
