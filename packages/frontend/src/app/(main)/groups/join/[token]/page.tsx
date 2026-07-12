import { Navigation } from "@/components/layout/Navigation";
import { ServiceFooter } from "@/components/layout/ServiceFooter";
import JoinGroupClient from "./JoinGroupClient";

export default async function JoinGroupPage({
  params,
}: {
  params: Promise<{ token: string }>;
}) {
  const { token } = await params;

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <JoinGroupClient token={token} />
      </main>
      <ServiceFooter />
    </div>
  );
}
