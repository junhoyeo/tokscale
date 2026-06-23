// Synthetic seed data for local development.
//
// Usage:
//   DATABASE_URL=postgresql://tokscale:tokscale@localhost:5432/tokscale \
//     bun run packages/frontend/scripts/seed-dev.ts
//
// Idempotent: re-running clears anything inserted with the seed marker
// `seed-dev@*` (devices, submissions) before re-creating it. Other rows
// (real users, real submissions) are left untouched, so it is safe to point
// at a DB that already has data.
//
// What it creates:
//   - 3 demo users (alice, bob, carol)
//   - 2 devices per user (laptop + desktop)
//   - 1 submissions row per user
//   - 14 daily_breakdown rows per device (~2 weeks of activity)
//   - 1 public group "Engineering Leaderboard" with all three users
//
// No PII: usernames are fictional, github_id values are negative so they
// cannot collide with any real GitHub user.

import {
  db,
  users,
  submittedDevices,
  submissions,
  dailyBreakdown,
  groups,
  groupMembers,
} from "../src/lib/db";
import { eq, or } from "drizzle-orm";
import { randomUUID } from "node:crypto";

const SEED_USERNAMES = ["seed-dev-alice", "seed-dev-bob", "seed-dev-carol"] as const;
const SEED_GROUP_SLUG = "seed-dev-engineering";

async function clearSeedData() {
  const existing = await db
    .select({ id: users.id })
    .from(users)
    .where(or(...SEED_USERNAMES.map((u) => eq(users.username, u))));

  if (existing.length > 0) {
    // FK cascades from users → submissions → daily_breakdown and
    // users → submitted_devices handle most of the graph automatically.
    for (const u of existing) {
      await db.delete(users).where(eq(users.id, u.id));
    }
  }

  await db.delete(groups).where(eq(groups.slug, SEED_GROUP_SLUG));
}

async function seedUser(
  username: string,
  displayName: string,
  githubId: number
): Promise<{ userId: string }> {
  const [created] = await db
    .insert(users)
    .values({
      username,
      displayName,
      githubId,
      avatarUrl: null,
      email: `${username}@example.com`,
    })
    .returning({ id: users.id });

  const userId = created.id;
  const now = new Date();

  const deviceKeys = ["laptop", "desktop"] as const;
  const deviceRows = await db
    .insert(submittedDevices)
    .values(
      deviceKeys.map((k) => ({
        userId,
        deviceKey: `seed-dev@${username}@${k}`,
        displayName: `${displayName}'s ${k}`,
        createdAt: now,
        updatedAt: now,
        lastSubmittedAt: now,
      }))
    )
    .returning({ id: submittedDevices.id, deviceKey: submittedDevices.deviceKey });

  const [submission] = await db
    .insert(submissions)
    .values({
      userId,
      totalTokens: 0,
      totalCost: "0",
      inputTokens: 0,
      outputTokens: 0,
      cacheCreationTokens: 0,
      cacheReadTokens: 0,
      reasoningTokens: 0,
      dateStart: "2026-05-01",
      dateEnd: "2026-05-14",
      sourcesUsed: ["claude", "codex"],
      modelsUsed: ["claude-sonnet-4", "codex-mini"],
      cliVersion: "2.1.3",
      submissionHash: randomUUID().replace(/-/g, "").slice(0, 32),
      schemaVersion: 2,
    })
    .returning({ id: submissions.id });

  // 14 days of activity split across both devices
  const dailyRows: Array<typeof dailyBreakdown.$inferInsert> = [];
  let totalTokens = 0;
  let totalCost = 0;
  for (let day = 0; day < 14; day++) {
    const date = `2026-05-${String(day + 1).padStart(2, "0")}`;
    for (const device of deviceRows) {
      const tokens = 50_000 + day * 5_000 + (device.deviceKey.endsWith("laptop") ? 0 : 15_000);
      const cost = (tokens / 1_000_000) * 3.0;
      const input = Math.floor(tokens * 0.7);
      const output = Math.floor(tokens * 0.3);
      dailyRows.push({
        submissionId: submission.id,
        submittedDeviceId: device.id,
        date,
        tokens,
        cost: cost.toFixed(4),
        inputTokens: input,
        outputTokens: output,
        timestampMs: new Date(`${date}T12:00:00Z`).getTime(),
        // modelBreakdown is keyed by modelId → tokens-only number.
        modelBreakdown: { "claude-sonnet-4": tokens },
        // sourceBreakdown is keyed by clientId → full client metrics blob.
        sourceBreakdown: {
          claude: {
            tokens,
            cost,
            input,
            output,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
            messages: 1,
          },
        },
      });
      totalTokens += tokens;
      totalCost += cost;
    }
  }
  if (dailyRows.length > 0) {
    await db.insert(dailyBreakdown).values(dailyRows);
  }

  // Reflect the rolled-up totals on the submissions row so reads match.
  await db
    .update(submissions)
    .set({
      totalTokens,
      totalCost: totalCost.toFixed(4),
      inputTokens: Math.floor(totalTokens * 0.7),
      outputTokens: Math.floor(totalTokens * 0.3),
    })
    .where(eq(submissions.id, submission.id));

  return { userId };
}

async function seedGroup(memberUserIds: string[]) {
  if (memberUserIds.length === 0) return;
  const [group] = await db
    .insert(groups)
    .values({
      name: "Engineering Leaderboard",
      slug: SEED_GROUP_SLUG,
      description: "Seed group for local dev",
      isPublic: true,
      createdBy: memberUserIds[0],
    })
    .returning({ id: groups.id });

  await db.insert(groupMembers).values(
    memberUserIds.map((userId, idx) => ({
      groupId: group.id,
      userId,
      role: idx === 0 ? ("owner" as const) : ("member" as const),
      invitedBy: memberUserIds[0],
    }))
  );
}

async function main() {
  if (!process.env.DATABASE_URL) {
    console.error("DATABASE_URL must be set");
    process.exit(1);
  }
  // Refuse to run against anything that doesn't look local. Production
  // database connection strings should not be passed to this seeder.
  const url = process.env.DATABASE_URL;
  if (!/localhost|127\.0\.0\.1|::1/.test(url)) {
    console.error(
      `seed-dev refuses to run against DATABASE_URL=${url}: only local hosts are allowed`
    );
    process.exit(1);
  }

  console.log("Clearing previous seed data...");
  await clearSeedData();

  console.log("Inserting users + devices + submissions + daily_breakdown...");
  const seeded: string[] = [];
  let githubId = -1000;
  for (const username of SEED_USERNAMES) {
    const displayName = username
      .replace("seed-dev-", "")
      .replace(/^[a-z]/, (c) => c.toUpperCase());
    const { userId } = await seedUser(username, displayName, githubId);
    seeded.push(userId);
    githubId -= 1;
  }

  console.log("Inserting demo group...");
  await seedGroup(seeded);

  console.log(`Done. Seeded ${seeded.length} users + 1 group.`);
  process.exit(0);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
