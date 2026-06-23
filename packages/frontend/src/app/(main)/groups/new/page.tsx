import { redirect } from "next/navigation";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import { getSession } from "@/lib/auth/session";
import CreateGroupClient from "./CreateGroupClient";

export default async function NewGroupPage() {
  const session = await getSession();

  if (!session) {
    redirect("/api/auth/github?returnTo=/groups/new");
  }

  return (
    <div
      style={{
        minHeight: "100vh",
        display: "flex",
        flexDirection: "column",
        backgroundColor: "var(--color-bg-default)",
      }}
    >
      <Navigation />
      <main className="main-container">
        <CreateGroupClient />
      </main>
      <Footer />
    </div>
  );
}
