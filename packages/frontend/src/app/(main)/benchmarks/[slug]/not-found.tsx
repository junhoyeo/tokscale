import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { BenchmarkNotFound } from "@/components/explore/BenchmarkDetailView";

export default function BenchmarkNotFoundPage() {
  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <BenchmarkNotFound />
      </main>
      <PublicFooter />
    </div>
  );
}
