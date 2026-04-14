import { redirect } from "next/navigation";
import { getSession } from "@/lib/auth/session";

export default async function ProfilePage() {
  const session = await getSession().catch(() => null);

  if (session) {
    redirect(`/u/${session.username}`);
  } else {
    redirect("/api/auth/github");
  }
}
