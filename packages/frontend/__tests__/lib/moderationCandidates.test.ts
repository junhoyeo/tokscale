import { afterEach, describe, expect, it, vi } from "vitest";
import { PgDialect } from "drizzle-orm/pg-core";
import type { SQL } from "drizzle-orm";
import { getModerationCandidates } from "@/lib/moderation/candidates";
import { applyModerationAction } from "@/lib/moderation/actions";
import { db } from "@/lib/db";

const dialect = new PgDialect();

vi.mock("@/lib/db", () => {
  return {
    db: {
      execute: vi.fn(),
      transaction: vi.fn(),
      select: vi.fn(),
    },
    users: {
      id: "users.id",
      username: "users.username",
      leaderboardHidden: "users.leaderboard_hidden",
      updatedAt: "users.updated_at",
    },
    moderationActions: {
      id: "moderation_actions.id",
      targetUserId: "moderation_actions.target_user_id",
      targetUsername: "moderation_actions.target_username",
      actorUserId: "moderation_actions.actor_user_id",
      actorUsername: "moderation_actions.actor_username",
      action: "moderation_actions.action",
      reason: "moderation_actions.reason",
      createdAt: "moderation_actions.created_at",
    },
  };
});

vi.mock("next/cache", () => ({
  revalidatePath: vi.fn(),
  revalidateTag: vi.fn(),
}));

vi.mock("@/lib/db/usernameLookup", () => ({
  USERNAME_LOOKUP_LIMIT: 20,
  getSingleUsernameMatch: vi.fn(),
  normalizeUsernameCacheKey: vi.fn((u: string) => u.toLowerCase()),
  revalidateUsernamePaths: vi.fn(),
  usernameEqualsIgnoreCase: vi.fn(),
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateUserGroupLeaderboards: vi.fn(),
}));

describe("getModerationCandidates - moderation action metadata exposure", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.restoreAllMocks();
  });

  it("exposes latestActorUsername, latestAction, and latestActionReason for system-withheld accounts and emits the latest_mod CTE query shape", async () => {
    const mockDbRow = {
      user_id: "user-1",
      username: "suspect_bot",
      avatar_url: null,
      leaderboard_hidden: true,
      total_tokens: "1000000000",
      total_cost: "50.0000",
      submit_count: 5,
      has_backfill: false,
      daily_tokens: "1000000000",
      near_duplicate_count: 0,
      slop_models: ["gpt-5-turbo"],
      slop_tokens: "1000000000",
      has_over_nested_entry: false,
      every_day_attributed: true,
      attributed_tokens: "1000000000",
      site_tokens: "5000000000",
      median_tokens: "500000",
      latest_actor_username: "system",
      latest_action: "hide",
      latest_action_reason: "Under investigation",
    };

    vi.mocked(db.execute).mockResolvedValueOnce([mockDbRow] as unknown as never);

    const candidates = await getModerationCandidates();

    expect(db.execute).toHaveBeenCalledTimes(1);
    const executedSql = dialect
      .sqlToQuery(vi.mocked(db.execute).mock.calls[0][0] as unknown as SQL)
      .sql.replace(/\s+/g, " ");
    expect(executedSql).toContain(
      "latest_mod AS ( SELECT DISTINCT ON (target_user_id) target_user_id, actor_username, action, reason FROM moderation_actions WHERE target_user_id IS NOT NULL ORDER BY target_user_id, created_at DESC, id DESC )"
    );
    expect(executedSql).toContain(
      "LEFT JOIN latest_mod lm ON lm.target_user_id = p.user_id"
    );
    expect(executedSql).toContain("lm.actor_username AS latest_actor_username");
    expect(executedSql).toContain("lm.action AS latest_action");
    expect(executedSql).toContain("lm.reason AS latest_action_reason");

    expect(candidates).toHaveLength(1);
    const candidate = candidates[0];
    expect(candidate.username).toBe("suspect_bot");
    expect(candidate.leaderboardHidden).toBe(true);
    expect(candidate.latestActorUsername).toBe("system");
    expect(candidate.latestAction).toBe("hide");
    expect(candidate.latestActionReason).toBe("Under investigation");
  });

  it("exposes admin actor username and reason for manually hidden accounts", async () => {
    const mockDbRow = {
      user_id: "user-2",
      username: "cheater_account",
      avatar_url: null,
      leaderboard_hidden: true,
      total_tokens: "2000000",
      total_cost: "10.0000",
      submit_count: 2,
      has_backfill: false,
      daily_tokens: "2000000",
      near_duplicate_count: 0,
      slop_models: [],
      slop_tokens: 0,
      has_over_nested_entry: false,
      every_day_attributed: true,
      attributed_tokens: "2000000",
      site_tokens: "5000000000",
      median_tokens: "500000",
      latest_actor_username: "admin_moderator",
      latest_action: "hide",
      latest_action_reason: "Abuse",
    };

    vi.mocked(db.execute).mockResolvedValueOnce([mockDbRow] as unknown as never);

    const candidates = await getModerationCandidates();

    expect(candidates).toHaveLength(1);
    const candidate = candidates[0];
    expect(candidate.username).toBe("cheater_account");
    expect(candidate.leaderboardHidden).toBe(true);
    expect(candidate.latestActorUsername).toBe("admin_moderator");
    expect(candidate.latestAction).toBe("hide");
    expect(candidate.latestActionReason).toBe("Abuse");
  });

  it("sets latest moderation metadata to null when no moderation action exists", async () => {
    const mockDbRow = {
      user_id: "user-3",
      username: "unreviewed_spoofer",
      avatar_url: null,
      leaderboard_hidden: true,
      total_tokens: "3000000",
      total_cost: "15.0000",
      submit_count: 1,
      has_backfill: false,
      daily_tokens: "3000000",
      near_duplicate_count: 0,
      slop_models: [],
      slop_tokens: 0,
      has_over_nested_entry: false,
      every_day_attributed: true,
      attributed_tokens: "3000000",
      site_tokens: "5000000000",
      median_tokens: "500000",
      latest_actor_username: null,
      latest_action: null,
      latest_action_reason: null,
    };

    vi.mocked(db.execute).mockResolvedValueOnce([mockDbRow] as unknown as never);

    const candidates = await getModerationCandidates();

    expect(candidates).toHaveLength(1);
    const candidate = candidates[0];
    expect(candidate.latestActorUsername).toBeNull();
    expect(candidate.latestAction).toBeNull();
    expect(candidate.latestActionReason).toBeNull();
  });

  it("includes tokens_per_msg CTE and maps max_tokens_per_message onto candidate", async () => {
    const mockDbRow = {
      user_id: "user-per-msg",
      username: "per_msg_spoofer",
      avatar_url: null,
      leaderboard_hidden: false,
      total_tokens: "1000000000",
      total_cost: "50.0000",
      submit_count: 1,
      has_backfill: false,
      daily_tokens: "1000000000",
      near_duplicate_count: 0,
      slop_models: [],
      slop_tokens: 0,
      has_over_nested_entry: false,
      every_day_attributed: true,
      attributed_tokens: "1000000000",
      site_tokens: "5000000000",
      median_tokens: "500000",
      latest_actor_username: null,
      latest_action: null,
      latest_action_reason: null,
      max_tokens_per_message: "123456789.5",
    };

    vi.mocked(db.execute).mockResolvedValueOnce([mockDbRow] as unknown as never);

    const candidates = await getModerationCandidates();

    expect(db.execute).toHaveBeenCalledTimes(1);
    const query = dialect.sqlToQuery(vi.mocked(db.execute).mock.calls[0][0] as unknown as SQL);
    const executedSql = query.sql.replace(/\s+/g, " ");

    expect(executedSql).toContain("tokens_per_msg AS (");
    expect(executedSql).toContain("LEFT JOIN tokens_per_msg tpm ON tpm.submission_id = p.submission_id");
    expect(executedSql).toContain("COALESCE(tpm.max_tokens_per_message, 0) AS max_tokens_per_message");
    expect(executedSql).toMatch(/OR max_tokens_per_message > \$\d+/);
    expect(query.params).toContain(100_000_000);
    expect(query.params).toContain(10_000_000);

    expect(candidates).toHaveLength(1);
    const candidate = candidates[0];
    expect(candidate.username).toBe("per_msg_spoofer");
    expect(candidate.maxTokensPerMessage).toBe(123456789.5);
    expect(candidate.score).toBeGreaterThanOrEqual(75);
    expect(candidate.signals).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: "usageLimit" }),
        expect.objectContaining({ key: "tokensPerMessage" }),
      ])
    );
  });
});

describe("applyModerationAction - admin override protection integration", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("records the authenticated administrator's username (trimmed) in moderationActions", async () => {
    const target = {
      id: "user-target-1",
      username: "AliceUser",
      leaderboardHidden: true,
    };

    interface InsertedAction {
      targetUserId: string;
      targetUsername: string;
      actorUserId: string;
      actorUsername: string;
      action: string;
      reason: string;
    }
    let insertedRecord: InsertedAction | null = null;

    (db.transaction as unknown as ReturnType<typeof vi.fn>).mockImplementation(
      async (callback: (tx: unknown) => Promise<unknown>) => {
        const tx = {
          update: vi.fn().mockReturnValue({
            set: vi.fn().mockReturnValue({
              where: vi.fn().mockReturnValue({
                returning: vi.fn().mockResolvedValue([{ id: target.id, username: target.username }]),
              }),
            }),
          }),
          insert: vi.fn().mockReturnValue({
            values: vi.fn().mockImplementation((val: InsertedAction) => {
              insertedRecord = val;
              return Promise.resolve();
            }),
          }),
        };
        return callback(tx);
      }
    );

    const result = await applyModerationAction({
      target,
      actorUserId: "admin-uuid-123",
      actorUsername: "  admin_lead  ",
      action: "unhide",
      reason: "Restored after review",
    });

    expect(result.changed).toBe(true);
    expect(result.leaderboardHidden).toBe(false);

    expect(insertedRecord).toEqual({
      targetUserId: "user-target-1",
      targetUsername: "AliceUser",
      actorUserId: "admin-uuid-123",
      actorUsername: "admin_lead",
      action: "unhide",
      reason: "Restored after review",
      createdAt: expect.any(Object),
    });

    // In Phase 2 admin override check:
    // const modActor = latestModAction?.actorUsername?.trim().toLowerCase();
    // const hasAdminOverride = latestModAction && latestModAction.action === "unhide" && Boolean(modActor && modActor !== "system");
    const record = insertedRecord as unknown as InsertedAction;
    expect(record).not.toBeNull();
    const modActor = record.actorUsername.trim().toLowerCase();
    const hasAdminOverride =
      record.action === "unhide" &&
      Boolean(modActor && modActor !== "system");
    expect(hasAdminOverride).toBe(true);
  });

  it("records hide action with authenticated admin username and sets leaderboardHidden true", async () => {
    const target = {
      id: "user-target-2",
      username: "BobCheater",
      leaderboardHidden: false,
    };

    interface InsertedAction {
      targetUserId: string;
      targetUsername: string;
      actorUserId: string;
      actorUsername: string;
      action: string;
      reason: string;
    }
    let insertedRecord: InsertedAction | null = null;

    (db.transaction as unknown as ReturnType<typeof vi.fn>).mockImplementation(
      async (callback: (tx: unknown) => Promise<unknown>) => {
        const tx = {
          update: vi.fn().mockReturnValue({
            set: vi.fn().mockReturnValue({
              where: vi.fn().mockReturnValue({
                returning: vi.fn().mockResolvedValue([{ id: target.id, username: target.username }]),
              }),
            }),
          }),
          insert: vi.fn().mockReturnValue({
            values: vi.fn().mockImplementation((val: InsertedAction) => {
              insertedRecord = val;
              return Promise.resolve();
            }),
          }),
        };
        return callback(tx);
      }
    );

    const result = await applyModerationAction({
      target,
      actorUserId: "admin-uuid-456",
      actorUsername: "mod_lead",
      action: "hide",
      reason: "Abuse",
    });

    expect(result.changed).toBe(true);
    expect(result.leaderboardHidden).toBe(true);
    expect(insertedRecord).toEqual({
      targetUserId: "user-target-2",
      targetUsername: "BobCheater",
      actorUserId: "admin-uuid-456",
      actorUsername: "mod_lead",
      action: "hide",
      reason: "Abuse",
      createdAt: expect.any(Object),
    });
  });

  it("throws an error if actorUsername or reason is empty or whitespace", async () => {
    const target = {
      id: "user-target-3",
      username: "Charlie",
      leaderboardHidden: false,
    };

    await expect(
      applyModerationAction({
        target,
        actorUserId: "admin-uuid-789",
        actorUsername: "   ",
        action: "hide",
        reason: "Abuse",
      })
    ).rejects.toThrow("Actor username is required");

    await expect(
      applyModerationAction({
        target,
        actorUserId: "admin-uuid-789",
        actorUsername: "admin_valid",
        action: "hide",
        reason: "   ",
      })
    ).rejects.toThrow("A reason is required");
  });
});
