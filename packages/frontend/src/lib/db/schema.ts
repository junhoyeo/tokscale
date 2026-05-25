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
    token: varchar("token", { length: 64 }).notNull().unique(),
    expiresAt: timestamp("expires_at", { withTimezone: true }).notNull(),
    source: varchar("source", { length: 10 }).notNull().default("web"),
    userAgent: text("user_agent"),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    // Planner picks the explicit non-unique idx over the unique-constraint
    // sibling (~89k vs 0 scans on prod 2026-05-25); keep both.
    index("idx_sessions_token").on(table.token),
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
    totalCost: decimal("total_cost", { precision: 12, scale: 4 }).notNull(),
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

    totalActiveTimeMs: bigint("total_active_time_ms", { mode: "number" }),
    longestContinuousMs: bigint("longest_continuous_ms", { mode: "number" }),
    maxConcurrentSessions: integer("max_concurrent_sessions"),
    sessionCount: integer("session_count"),

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
    cost: decimal("cost", { precision: 10, scale: 4 }).notNull(),
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
export type Group = typeof groups.$inferSelect;
export type NewGroup = typeof groups.$inferInsert;
export type GroupMember = typeof groupMembers.$inferSelect;
export type NewGroupMember = typeof groupMembers.$inferInsert;
export type GroupInvite = typeof groupInvites.$inferSelect;
export type NewGroupInvite = typeof groupInvites.$inferInsert;
