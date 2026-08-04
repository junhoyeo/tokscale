import {
  pgTable,
  uuid,
  varchar,
  text,
  boolean,
  timestamp,
  bigint,
  decimal,
  date,
  jsonb,
  integer,
  index,
  unique,
  uniqueIndex,
  primaryKey,
} from "drizzle-orm/pg-core";
import { relations } from "drizzle-orm";
import {
  USERS_USERNAME_LOWER_UNIQUE_INDEX,
  usernameLowerExpression,
} from "./usernameIndex";

// ============================================================================
// USERS
// ============================================================================
export const users = pgTable(
  "users",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    githubId: integer("github_id").notNull().unique(),
    username: varchar("username", { length: 39 }).notNull().unique(),
    displayName: varchar("display_name", { length: 255 }),
    avatarUrl: text("avatar_url"),
    email: varchar("email", { length: 255 }),
    /**
     * Excludes the user from leaderboard RANKINGS only. Their profile, badge
     * and embeds stay public, and their usage still counts toward site-wide
     * totals — see lib/leaderboard/getLeaderboard.ts for exactly which queries
     * honour this and which deliberately do not.
     *
     * Reversible by design: moderation_actions keeps the full hide/unhide
     * history, so this column is current state, not the record.
     *
     * Intentionally unindexed. Nearly every row passes `NOT leaderboard_hidden`,
     * so an index has no selectivity to offer; the check rides the existing
     * join against users.
     */
    leaderboardHidden: boolean("leaderboard_hidden").notNull().default(false),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    // Both indexes on username are intentional: the prod planner consistently
    // picks the explicit non-unique idx_users_username (30k scans) over the
    // unique-constraint sibling (0 scans). Removing this is a real re-plan
    // event; don't.
    index("idx_users_username").on(table.username),
    uniqueIndex(USERS_USERNAME_LOWER_UNIQUE_INDEX).on(
      usernameLowerExpression(table.username)
    ),
    index("idx_users_github_id").on(table.githubId),
  ]
);

export const usersRelations = relations(users, ({ many }) => ({
  sessions: many(sessions),
  apiTokens: many(apiTokens),
  submissions: many(submissions),
  submittedDevices: many(submittedDevices),
  groupMemberships: many(groupMembers, { relationName: "memberUser" }),
  createdGroups: many(groups, { relationName: "groupCreator" }),
  createdGroupInvites: many(groupInvites, { relationName: "groupInviteCreator" }),
  moderationActionsReceived: many(moderationActions, { relationName: "moderationTarget" }),
  moderationActionsTaken: many(moderationActions, { relationName: "moderationActor" }),
}));

// ============================================================================
// SESSIONS
// ============================================================================
export const sessions = pgTable(
  "sessions",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    tokenHash: varchar("token_hash", { length: 64 }).notNull().unique(),
    expiresAt: timestamp("expires_at", { withTimezone: true }).notNull(),
    source: varchar("source", { length: 10 }).notNull().default("web"),
    userAgent: text("user_agent"),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    index("idx_sessions_token_hash").on(table.tokenHash),
    index("idx_sessions_user_id").on(table.userId),
    index("idx_sessions_expires_at").on(table.expiresAt),
  ]
);

export const sessionsRelations = relations(sessions, ({ one }) => ({
  user: one(users, {
    fields: [sessions.userId],
    references: [users.id],
  }),
}));

// ============================================================================
// API TOKENS
// ============================================================================
export const apiTokens = pgTable(
  "api_tokens",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    token: varchar("token", { length: 64 }).notNull().unique(),
    name: varchar("name", { length: 100 }).notNull(),
    lastUsedAt: timestamp("last_used_at", { withTimezone: true }),
    expiresAt: timestamp("expires_at", { withTimezone: true }),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    // Planner picks the explicit non-unique idx (~27k scans) over the
    // unique-constraint sibling (0 scans); keep both.
    index("idx_api_tokens_token").on(table.token),
    index("idx_api_tokens_user_id").on(table.userId),
    unique("api_tokens_user_name_unique").on(table.userId, table.name),
  ]
);

export const apiTokensRelations = relations(apiTokens, ({ one }) => ({
  user: one(users, {
    fields: [apiTokens.userId],
    references: [users.id],
  }),
}));

// ============================================================================
// DEVICE CODES
// ============================================================================
export const deviceCodes = pgTable(
  "device_codes",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    deviceCode: varchar("device_code", { length: 32 }).notNull().unique(),
    userCode: varchar("user_code", { length: 9 }).notNull().unique(),
    userId: uuid("user_id").references(() => users.id, { onDelete: "cascade" }),
    deviceName: varchar("device_name", { length: 100 }),
    expiresAt: timestamp("expires_at", { withTimezone: true }).notNull(),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    // The .unique() siblings exist for device_code / user_code but the
    // planner picks the explicit non-unique indexes; keep them.
    index("idx_device_codes_device_code").on(table.deviceCode),
    index("idx_device_codes_user_code").on(table.userCode),
    // idx_device_codes_user_id covers the FK so cascade-delete of a user
    // doesn't seq scan this table.
    index("idx_device_codes_user_id").on(table.userId),
    index("idx_device_codes_expires_at").on(table.expiresAt),
  ]
);

// ============================================================================
// SUBMISSIONS
// ============================================================================
export const submissions = pgTable(
  "submissions",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),

    totalTokens: bigint("total_tokens", { mode: "number" }).notNull(),
    totalCost: decimal("total_cost", { precision: 18, scale: 4 }).notNull(),
    inputTokens: bigint("input_tokens", { mode: "number" }).notNull(),
    outputTokens: bigint("output_tokens", { mode: "number" }).notNull(),
    cacheCreationTokens: bigint("cache_creation_tokens", { mode: "number" })
      .notNull()
      .default(0),
    cacheReadTokens: bigint("cache_read_tokens", { mode: "number" })
      .notNull()
      .default(0),
    reasoningTokens: bigint("reasoning_tokens", { mode: "number" })
      .notNull()
      .default(0),

    dateStart: date("date_start").notNull(),
    dateEnd: date("date_end").notNull(),

    sourcesUsed: text("sources_used").array().notNull(),
    modelsUsed: text("models_used").array().notNull(),

    cliVersion: varchar("cli_version", { length: 20 }),
    submissionHash: varchar("submission_hash", { length: 64 }),
    submitCount: integer("submit_count").notNull().default(1),
    /** 0=legacy (no timestamps), 1=timestamp-aware CLI */
    schemaVersion: integer("schema_version").notNull().default(0),
    /**
     * True once ANY accepted submission for this user carried a
     * submission-level `provenance.origin === "backfill"` tag (e.g. from
     * `tokscale import`). Sticky: later live CLI submits never reset it,
     * because the merged totals still include the imported history.
     */
    hasBackfill: boolean("has_backfill").notNull().default(false),

    totalActiveTimeMs: bigint("total_active_time_ms", { mode: "number" }),
    longestContinuousMs: bigint("longest_continuous_ms", { mode: "number" }),
    maxConcurrentSessions: integer("max_concurrent_sessions"),
    sessionCount: integer("session_count"),

    mcpServers: jsonb("mcp_servers").$type<string[]>(),

    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    index("idx_submissions_created_at").on(table.createdAt),
    // idx_submissions_leaderboard serves every user_id lookup as a left-prefix
    // index, so a plain idx_submissions_user_id would be redundant. Do not
    // re-add it without first checking pg_stat_user_indexes on the composite.
    index("idx_submissions_leaderboard").on(table.userId, table.totalTokens, table.totalCost, table.createdAt),
    unique("submissions_user_id_unique").on(table.userId),
  ]
);

export const submissionsRelations = relations(submissions, ({ one, many }) => ({
  user: one(users, {
    fields: [submissions.userId],
    references: [users.id],
  }),
  dailyBreakdown: many(dailyBreakdown),
}));

// ============================================================================
// SUBMITTED DEVICES
// ============================================================================
export const submittedDevices = pgTable(
  "submitted_devices",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    deviceKey: varchar("device_key", { length: 96 }).notNull(),
    displayName: varchar("display_name", { length: 120 }),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    lastSubmittedAt: timestamp("last_submitted_at", { withTimezone: true }),

    /**
     * Per-device session-shape metrics, kept as monotonic high-water marks.
     *
     * These mirror the identically-named columns on `submissions`, but at the
     * scope the CLI actually measures: one machine's local session files. The
     * submission-level values are DERIVED from these (SUM for additive metrics,
     * MAX for shape metrics) so a second device no longer overwrites the first.
     *
     * `totalActiveTimeMs` here comes from the CLI's `timeMetrics`, which sums
     * raw interval durations and is therefore TIMEZONE-INVARIANT. The daily
     * `active_time_ms` rows are not: they apportion each interval across LOCAL
     * calendar days, so re-scanning under a different TZ re-splits them and
     * their monotonic merge inflates the sum. Deriving the submission total
     * from these columns instead of SUM(daily) avoids that.
     */
    totalActiveTimeMs: bigint("total_active_time_ms", { mode: "number" }),
    longestContinuousMs: bigint("longest_continuous_ms", { mode: "number" }),
    maxConcurrentSessions: integer("max_concurrent_sessions"),
    sessionCount: integer("session_count"),
  },
  (table) => [
    index("idx_submitted_devices_user_id").on(table.userId),
    unique("submitted_devices_user_device_key_unique").on(table.userId, table.deviceKey),
  ]
);

export const submittedDevicesRelations = relations(submittedDevices, ({ one, many }) => ({
  user: one(users, {
    fields: [submittedDevices.userId],
    references: [users.id],
  }),
  dailyBreakdown: many(dailyBreakdown),
}));

// ============================================================================
// DAILY BREAKDOWN
// ============================================================================
export const dailyBreakdown = pgTable(
  "daily_breakdown",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    submittedDeviceId: uuid("submitted_device_id")
      .notNull()
      .references(() => submittedDevices.id, { onDelete: "cascade" }),

    date: date("date").notNull(),
    tokens: bigint("tokens", { mode: "number" }).notNull(),
    cost: decimal("cost", { precision: 14, scale: 4 }).notNull(),
    inputTokens: bigint("input_tokens", { mode: "number" }).notNull(),
    outputTokens: bigint("output_tokens", { mode: "number" }).notNull(),
    /** Unix ms timestamp of earliest message in this UTC day bucket. NULL for legacy data. */
    timestampMs: bigint("timestamp_ms", { mode: "number" }),

    sourceBreakdown: jsonb("source_breakdown").$type<
      Record<
        string,
        {
          tokens: number;
          cost: number;
          input: number;
          output: number;
          cacheRead: number;
          cacheWrite: number;
          reasoning: number;
          messages: number;
          models: Record<string, {
            tokens: number;
            cost: number;
            input: number;
            output: number;
            cacheRead: number;
            cacheWrite: number;
            reasoning: number;
            messages: number;
          }>;
          provenance?: {
            schemaVersion: number;
            messageCount: number;
            modelCount: number;
            /**
             * "backfill" when this client's contribution came from a
             * backfill-origin submission (`tokscale import`); absent/"cli"
             * for locally-scanned usage.
             */
            origin?: "cli" | "backfill";
          };
          modelId?: string;
        }
      >
    >(),
    /** Total active coding time in this UTC day bucket (milliseconds). NULL for legacy data. */
    activeTimeMs: bigint("active_time_ms", { mode: "number" }),
  },
  (table) => [
    index("idx_daily_breakdown_submission_id").on(table.submissionId),
    index("idx_daily_breakdown_submitted_device_id").on(table.submittedDeviceId),
    index("idx_daily_breakdown_date").on(table.date),
    unique("daily_breakdown_submission_device_date_unique").on(
      table.submissionId,
      table.submittedDeviceId,
      table.date
    ),
  ]
);

export const dailyBreakdownRelations = relations(dailyBreakdown, ({ one }) => ({
  submission: one(submissions, {
    fields: [dailyBreakdown.submissionId],
    references: [submissions.id],
  }),
  submittedDevice: one(submittedDevices, {
    fields: [dailyBreakdown.submittedDeviceId],
    references: [submittedDevices.id],
  }),
}));

// ============================================================================
// DAILY BREAKDOWN REPORTED (ratchet-inflation shadow, Phase 4a)
// ============================================================================
/**
 * Unguarded latest observations for reported (device, date, client) cells.
 *
 * Phase 4a of docs/ratchet-inflation-recovery.md. **Nothing reads this table.**
 * It records explicit cell reports that the monotonic merge on
 * `daily_breakdown` throws away. It is not a whole-scan snapshot and has no
 * reader or recovery behavior today.
 *
 * Merge semantics: last-write-wins on conflict. No `GREATEST`, no regression
 * guard, no alias-fold normalisation. A `--since` scan that omits a cell does
 * not touch that cell's row — absence is not zero. A future recovery workflow
 * needs client-declared authoritative coverage and snapshot generations or
 * tombstones before an omitted cell can be treated as absent.
 *
 * **Never backfill this from `daily_breakdown`.** The stored rows are the
 * inflated values this table exists to contradict; seeding from them would
 * leave Phase 4b with nothing to heal.
 *
 * `origin` is a plain column, not part of the primary key. Unlike
 * `submitted_device_client_totals`, a later scan of either origin replaces the
 * previous explicit observation for that (device, date, client) outright — it
 * is a per-cell LWW table, not a complete payload snapshot or high-water.
 */
export const dailyBreakdownReported = pgTable(
  "daily_breakdown_reported",
  {
    submittedDeviceId: uuid("submitted_device_id")
      .notNull()
      .references(() => submittedDevices.id, { onDelete: "cascade" }),
    date: date("date").notNull(),
    /** Canonical client id (post alias normalization, e.g. "kilo" not "kilocode"). */
    client: varchar("client", { length: 128 }).notNull(),

    tokens: bigint("tokens", { mode: "number" }).notNull(),
    cost: decimal("cost", { precision: 14, scale: 4 }).notNull(),
    input: bigint("input", { mode: "number" }).notNull(),
    output: bigint("output", { mode: "number" }).notNull(),
    /** Day-level active time, repeated on each client row for that date. */
    activeTimeMs: bigint("active_time_ms", { mode: "number" }),
    /** "cli" for locally-scanned usage, "backfill" for `tokscale import`. */
    origin: varchar("origin", { length: 16 }).notNull(),

    reportedAt: timestamp("reported_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    primaryKey({
      columns: [table.submittedDeviceId, table.date, table.client],
    }),
    index("idx_daily_breakdown_reported_device_date").on(
      table.submittedDeviceId,
      table.date
    ),
  ]
);

export const dailyBreakdownReportedRelations = relations(
  dailyBreakdownReported,
  ({ one }) => ({
    submittedDevice: one(submittedDevices, {
      fields: [dailyBreakdownReported.submittedDeviceId],
      references: [submittedDevices.id],
    }),
  })
);

// ============================================================================
// SUBMITTED DEVICE CLIENT TOTALS (ratchet-inflation census, Phase 1)
// ============================================================================
/**
 * Per-device, per-client, per-bucket token/cost HIGH-WATER marks.
 *
 * Phase 1 of docs/ratchet-inflation-recovery.md. **Nothing reads this table.**
 * It exists so the inflation that `SUM(daily_breakdown.tokens)` accumulates can
 * be measured against a bucket-level high-water reconstruction of the same
 * history, on live traffic, before any read path is switched over (Phase 2).
 *
 * Merge semantics: `GREATEST` on conflict, mirroring the per-device session
 * metrics on `submitted_devices`. A `--clients`/`--date` filtered submit
 * reports only a slice of a bucket and must never lower the stored value.
 *
 * **This table must never be backfilled from `daily_breakdown`.** Seeding
 * `tokens_highwater` with `SUM(daily in bucket)` would seed it with the
 * INFLATED value; a later truthful full scan reports the true, lower total and
 * `GREATEST(inflated, true)` keeps the inflated one permanently. The table
 * starts empty and is filled only by incoming payloads — the warm-up is forced
 * by the merge semantics, not a convenience to optimise away.
 *
 * `origin` is part of the primary key and is load-bearing. `getSubmitDevice`
 * in the submit route falls back to `LEGACY_SUBMIT_DEVICE_KEY` when a payload
 * omits `device`, so a `tokscale import` backfill and a legacy CLI submit land
 * on the SAME `submitted_devices` row. Keyed without origin, `GREATEST` would
 * take the max of imported and locally-scanned history instead of their sum,
 * silently dropping whichever is smaller.
 *
 * The flip side of that key, also KNOWN and also left for Phase 2: `origin` is
 * caller-controlled (`provenance.origin` on the payload), and the two origins
 * ADD here while `daily_breakdown` MERGES. `mergeClientBreakdownsWithRegression
 * Guard` REPLACES a client's day entry rather than summing it, so resubmitting
 * the same history twice — once tagged `backfill`, once untagged — leaves the
 * daily rows at H but leaves this table with two rows summing to 2H. Additivity
 * across origins is only correct when the two bodies of history are DISJOINT,
 * which is the honest case (import old history, scan recent history) but is not
 * enforced. Phase 2 must not read `SUM(tokens_highwater)` across origins
 * without accounting for this; Phase 1.5 surfaces it as a served/high-water
 * ratio near 0.5, the same signature as the adoption case below.
 *
 * KNOWN, and deliberately not fixed here — Phase 2 inherits it. The submit
 * route's legacy-adoption path re-parents a user's `daily_breakdown` rows from
 * the LEGACY device to their first device-aware device, so those days are
 * counted once. This table has no equivalent re-parenting: a user who submits
 * from a legacy CLI and later from a device-aware one ends up with the same
 * history recorded under BOTH device ids, so a naive
 * `SUM(tokens_highwater)` over their devices double-counts it. Phase 1 is
 * inert so nothing is wrong today, and Phase 1.5 is exactly what surfaces it —
 * those accounts show a served/high-water ratio near 0.5. Phase 2 must handle
 * it (adopt the rows the same way, or aggregate per user+client+bucket)
 * BEFORE it reads from here.
 *
 * Column types are chosen deliberately (see the `LEAST(...)` clamps in the
 * submit route): `tokens_highwater` is bigint because a bucket total is a sum
 * of day totals, and any aggregate reading it back must clamp before casting.
 * `cost_highwater` is numeric(18,4) — wider than `daily_breakdown.cost`'s
 * numeric(14,4), so a month-wide sum of day costs that already fit the
 * narrower column cannot overflow this one.
 */
export const submittedDeviceClientTotals = pgTable(
  "submitted_device_client_totals",
  {
    submittedDeviceId: uuid("submitted_device_id")
      .notNull()
      .references(() => submittedDevices.id, { onDelete: "cascade" }),
    /** Canonical client id (post alias normalization, e.g. "kilo" not "kilocode"). */
    client: varchar("client", { length: 128 }).notNull(),
    /** "cli" for locally-scanned usage, "backfill" for `tokscale import`. */
    origin: varchar("origin", { length: 16 }).notNull(),
    /** Only "month" is written today; the column keeps Phase 3 open. */
    bucketWidth: varchar("bucket_width", { length: 8 }).notNull(),
    /** Stable bucket label: `YYYY-MM` for bucket_width = "month". */
    bucketKey: varchar("bucket_key", { length: 16 }).notNull(),

    tokensHighwater: bigint("tokens_highwater", { mode: "number" })
      .notNull()
      .default(0),
    costHighwater: decimal("cost_highwater", { precision: 18, scale: 4 })
      .notNull()
      .default("0"),

    updatedAt: timestamp("updated_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    primaryKey({
      columns: [
        table.submittedDeviceId,
        table.client,
        table.origin,
        table.bucketWidth,
        table.bucketKey,
      ],
    }),
  ]
);

export const submittedDeviceClientTotalsRelations = relations(
  submittedDeviceClientTotals,
  ({ one }) => ({
    submittedDevice: one(submittedDevices, {
      fields: [submittedDeviceClientTotals.submittedDeviceId],
      references: [submittedDevices.id],
    }),
  })
);

// ============================================================================
// RATCHET CENSUS WORK (durable post-commit high-water writes)
// ============================================================================
/**
 * One durable deferred high-water write. It is registered in the submit
 * transaction, then replayed and deleted only after its idempotent upsert
 * succeeds. This lets a later submit recover work abandoned by an interrupted
 * request without mistaking that gap for a stable census divergence.
 */
export const ratchetCensusWork = pgTable(
  "ratchet_census_work",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    submittedDeviceId: uuid("submitted_device_id")
      .notNull()
      .references(() => submittedDevices.id, { onDelete: "cascade" }),
    buckets: jsonb("buckets").$type<
      Array<{
        client: string;
        origin: "cli" | "backfill";
        bucketWidth: string;
        bucketKey: string;
        tokens: number;
        cost: number;
      }>
    >().notNull(),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [index("idx_ratchet_census_work_submission_id").on(table.submissionId)]
);

// ============================================================================
// GROUPS
// ============================================================================
export const groupRoles = ["owner", "admin", "member"] as const;
export type GroupRole = (typeof groupRoles)[number];

export const groupInviteStatuses = ["pending", "accepted", "declined", "expired"] as const;
export type GroupInviteStatus = (typeof groupInviteStatuses)[number];

export const groups = pgTable(
  "groups",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    name: varchar("name", { length: 100 }).notNull(),
    slug: varchar("slug", { length: 100 }).notNull().unique(),
    description: text("description"),
    avatarUrl: text("avatar_url"),
    isPublic: boolean("is_public").notNull().default(true),
    createdBy: uuid("created_by")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    index("idx_groups_created_by").on(table.createdBy),
    index("idx_groups_visibility_updated").on(table.isPublic, table.updatedAt),
  ]
);

export const groupsRelations = relations(groups, ({ one, many }) => ({
  creator: one(users, {
    fields: [groups.createdBy],
    references: [users.id],
    relationName: "groupCreator",
  }),
  members: many(groupMembers),
  invites: many(groupInvites),
}));

export const groupMembers = pgTable(
  "group_members",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    groupId: uuid("group_id")
      .notNull()
      .references(() => groups.id, { onDelete: "cascade" }),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    role: varchar("role", { length: 10 }).notNull().default("member").$type<GroupRole>(),
    invitedBy: uuid("invited_by").references(() => users.id, { onDelete: "set null" }),
    joinedAt: timestamp("joined_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    index("idx_group_members_user_id").on(table.userId),
    // FK coverage: cascade-delete of an inviter does a seq scan without this.
    index("idx_group_members_invited_by").on(table.invitedBy),
    unique("group_members_group_user_unique").on(table.groupId, table.userId),
  ]
);

export const groupMembersRelations = relations(groupMembers, ({ one }) => ({
  group: one(groups, {
    fields: [groupMembers.groupId],
    references: [groups.id],
  }),
  user: one(users, {
    fields: [groupMembers.userId],
    references: [users.id],
    relationName: "memberUser",
  }),
  inviter: one(users, {
    fields: [groupMembers.invitedBy],
    references: [users.id],
    relationName: "memberInviter",
  }),
}));

export const groupInvites = pgTable(
  "group_invites",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    groupId: uuid("group_id")
      .notNull()
      .references(() => groups.id, { onDelete: "cascade" }),
    invitedUsername: varchar("invited_username", { length: 39 }),
    invitedUsernameNormalized: varchar("invited_username_normalized", { length: 39 }),
    invitedUserId: uuid("invited_user_id").references(() => users.id, { onDelete: "cascade" }),
    invitedBy: uuid("invited_by")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    role: varchar("role", { length: 10 }).notNull().default("member").$type<GroupRole>(),
    status: varchar("status", { length: 10 }).notNull().default("pending").$type<GroupInviteStatus>(),
    tokenHash: varchar("token_hash", { length: 64 }).notNull().unique(),
    expiresAt: timestamp("expires_at", { withTimezone: true }).notNull(),
    acceptedAt: timestamp("accepted_at", { withTimezone: true }),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    index("idx_group_invites_group_status").on(table.groupId, table.status),
    index("idx_group_invites_invited_user_status").on(table.invitedUserId, table.status),
    index("idx_group_invites_invited_username_status").on(
      table.invitedUsernameNormalized,
      table.status
    ),
    index("idx_group_invites_expires_at").on(table.expiresAt),
    // FK coverage: cascade-delete of an inviter does a seq scan without this.
    index("idx_group_invites_invited_by").on(table.invitedBy),
  ]
);

export const groupInvitesRelations = relations(groupInvites, ({ one }) => ({
  group: one(groups, {
    fields: [groupInvites.groupId],
    references: [groups.id],
  }),
  invitedUser: one(users, {
    fields: [groupInvites.invitedUserId],
    references: [users.id],
    relationName: "groupInviteTarget",
  }),
  inviter: one(users, {
    fields: [groupInvites.invitedBy],
    references: [users.id],
    relationName: "groupInviteCreator",
  }),
}));

// ============================================================================
// MODERATION
// ============================================================================
export const MODERATION_ACTION_TYPES = ["hide", "unhide"] as const;
export type ModerationAction = (typeof MODERATION_ACTION_TYPES)[number];

/**
 * Append-only log of every leaderboard hide/unhide.
 *
 * `users.leaderboard_hidden` is current state; this is the record of how it got
 * there. Rows are never updated or deleted, so a sequence of hide → unhide →
 * hide stays fully reconstructible — which is the point of preferring a
 * reversible flag over deleting an account.
 */
export const moderationActions = pgTable(
  "moderation_actions",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    // User accounts are deletable, while this record is intentionally not.
    // The immutable names preserve a useful audit trail after either account
    // has gone away; nullable FKs retain relational lookup while present.
    targetUserId: uuid("target_user_id").references(() => users.id, {
      onDelete: "set null",
    }),
    targetUsername: varchar("target_username", { length: 39 }).notNull(),
    actorUserId: uuid("actor_user_id").references(() => users.id, {
      onDelete: "set null",
    }),
    actorUsername: varchar("actor_username", { length: 39 }).notNull(),
    action: varchar("action", { length: 10 }).notNull().$type<ModerationAction>(),
    /** Free-text justification, required at the API layer. */
    reason: text("reason").notNull(),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    // Serves the per-user history panel on the review screen.
    index("idx_moderation_actions_target_created").on(
      table.targetUserId,
      table.createdAt
    ),
    // FK coverage: nulling a deleted actor does a seq scan without this.
    index("idx_moderation_actions_actor").on(table.actorUserId),
  ]
);

export const moderationActionsRelations = relations(moderationActions, ({ one }) => ({
  targetUser: one(users, {
    fields: [moderationActions.targetUserId],
    references: [users.id],
    relationName: "moderationTarget",
  }),
  actorUser: one(users, {
    fields: [moderationActions.actorUserId],
    references: [users.id],
    relationName: "moderationActor",
  }),
}));

// ============================================================================
// TYPE EXPORTS
// ============================================================================
export type User = typeof users.$inferSelect;
export type NewUser = typeof users.$inferInsert;
export type Session = typeof sessions.$inferSelect;
export type NewSession = typeof sessions.$inferInsert;
export type ApiToken = typeof apiTokens.$inferSelect;
export type NewApiToken = typeof apiTokens.$inferInsert;
export type DeviceCode = typeof deviceCodes.$inferSelect;
export type NewDeviceCode = typeof deviceCodes.$inferInsert;
export type Submission = typeof submissions.$inferSelect;
export type NewSubmission = typeof submissions.$inferInsert;
export type SubmittedDevice = typeof submittedDevices.$inferSelect;
export type NewSubmittedDevice = typeof submittedDevices.$inferInsert;
export type DailyBreakdown = typeof dailyBreakdown.$inferSelect;
export type NewDailyBreakdown = typeof dailyBreakdown.$inferInsert;
export type SubmittedDeviceClientTotal = typeof submittedDeviceClientTotals.$inferSelect;
export type NewSubmittedDeviceClientTotal = typeof submittedDeviceClientTotals.$inferInsert;
export type Group = typeof groups.$inferSelect;
export type NewGroup = typeof groups.$inferInsert;
export type GroupMember = typeof groupMembers.$inferSelect;
export type NewGroupMember = typeof groupMembers.$inferInsert;
export type GroupInvite = typeof groupInvites.$inferSelect;
export type NewGroupInvite = typeof groupInvites.$inferInsert;
export type ModerationActionRow = typeof moderationActions.$inferSelect;
export type NewModerationActionRow = typeof moderationActions.$inferInsert;
