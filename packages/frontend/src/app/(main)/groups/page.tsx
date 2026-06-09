import { redirect } from "next/navigation";

// The old /groups listing page was consolidated into /leaderboard?view=groups
// so groups stops appearing as a separate top-level nav tab. Per-group detail
// (/groups/[slug]), creation (/groups/new), and invite acceptance
// (/groups/join/[token]) still live under /groups and are unaffected.
export default function GroupsRedirect() {
  redirect("/leaderboard?view=groups");
}
