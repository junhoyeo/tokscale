import { redirect } from "next/navigation";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { getSession } from "@/lib/auth/session";
import CreateGroupClient from "./CreateGroupClient";

export default async function NewGroupPage() {
  const session = await getSession();

  if (!session) {
    redirect("/api/auth/github?returnTo=/groups/new");
  }

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <CreateGroupClient />
      </main>
      <PublicFooter />
    </div>
  );
}
