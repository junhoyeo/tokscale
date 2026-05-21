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
    isAdmin: boolean("is_admin").notNull().default(false),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
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
  devices: many(devices),
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
    index("idx_device_codes_device_code").on(table.deviceCode),
    index("idx_device_codes_user_code").on(table.userCode),
    index("idx_device_codes_expires_at").on(table.expiresAt),
  ]
);

// ============================================================================
// DEVICES
// ============================================================================
export const devices = pgTable(
  "devices",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    /** CLI-generated stable UUID, or the sentinel "legacy". */
    deviceId: varchar("device_id", { length: 64 }).notNull(),
    name: varchar("name", { length: 100 }).notNull(),
    hostname: varchar("hostname", { length: 255 }),
    os: varchar("os", { length: 32 }),
    cliVersion: varchar("cli_version", { length: 20 }),
    lastSeenAt: timestamp("last_seen_at", { withTimezone: true }),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    index("idx_devices_user_id").on(table.userId),
    unique("devices_user_device_unique").on(table.userId, table.deviceId),
  ]
);

export const devicesRelations = relations(devices, ({ one, many }) => ({
  user: one(users, {
    fields: [devices.userId],
    references: [users.id],
  }),
  dailyBreakdown: many(dailyBreakdown),
}));

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

    status: varchar("status", { length: 20 }).notNull().default("verified"),

    cliVersion: varchar("cli_version", { length: 20 }),
    submissionHash: varchar("submission_hash", { length: 64 }),
    submitCount: integer("submit_count").notNull().default(1),
    /** 0=legacy (no timestamps), 1=timestamp-aware CLI */
    schemaVersion: integer("schema_version").notNull().default(0),

    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (table) => [
    index("idx_submissions_user_id").on(table.userId),
    index("idx_submissions_status").on(table.status),
    index("idx_submissions_total_tokens").on(table.totalTokens),
    index("idx_submissions_created_at").on(table.createdAt),
    index("idx_submissions_date_range").on(table.dateStart, table.dateEnd),
    index("idx_submissions_leaderboard").on(table.userId, table.totalTokens, table.totalCost, table.createdAt),
    unique("submissions_user_id_unique").on(table.userId),
    unique("submissions_user_hash_unique").on(table.userId, table.submissionHash),
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
// DAILY BREAKDOWN
// ============================================================================
export const dailyBreakdown = pgTable(
  "daily_breakdown",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    deviceId: uuid("device_id")
      .notNull()
      .references(() => devices.id, { onDelete: "cascade" }),

    date: date("date").notNull(),
    tokens: bigint("tokens", { mode: "number" }).notNull(),
    cost: decimal("cost", { precision: 10, scale: 4 }).notNull(),
    inputTokens: bigint("input_tokens", { mode: "number" }).notNull(),
    outputTokens: bigint("output_tokens", { mode: "number" }).notNull(),
    /** Unix ms timestamp of earliest message in this UTC day bucket. NULL for legacy data. */
    timestampMs: bigint("timestamp_ms", { mode: "number" }),

    providerBreakdown: jsonb("provider_breakdown").$type<
      Record<string, number>
    >(),
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
    modelBreakdown: jsonb("model_breakdown").$type<Record<string, number>>(),
  },
  (table) => [
    index("idx_daily_breakdown_submission_id").on(table.submissionId),
    index("idx_daily_breakdown_device_id").on(table.deviceId),
    index("idx_daily_breakdown_date").on(table.date),
    unique("daily_breakdown_submission_device_date_unique").on(
      table.submissionId,
      table.deviceId,
      table.date
    ),
  ]
);

export const dailyBreakdownRelations = relations(dailyBreakdown, ({ one }) => ({
  submission: one(submissions, {
    fields: [dailyBreakdown.submissionId],
    references: [submissions.id],
  }),
  device: one(devices, {
    fields: [dailyBreakdown.deviceId],
    references: [devices.id],
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
export type Device = typeof devices.$inferSelect;
export type NewDevice = typeof devices.$inferInsert;
export type Submission = typeof submissions.$inferSelect;
export type NewSubmission = typeof submissions.$inferInsert;
export type DailyBreakdown = typeof dailyBreakdown.$inferSelect;
export type NewDailyBreakdown = typeof dailyBreakdown.$inferInsert;
