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
} from "drizzle-orm/pg-core";
import { relations } from "drizzle-orm";

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
    index("idx_users_github_id").on(table.githubId),
  ]
);

export const usersRelations = relations(users, ({ many }) => ({
  sessions: many(sessions),
  apiTokens: many(apiTokens),
  submissions: many(submissions),
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

    dateStart: date("date_start").notNull(),
    dateEnd: date("date_end").notNull(),

    sourcesUsed: text("sources_used").array().notNull(),
    modelsUsed: text("models_used").array().notNull(),

    status: varchar("status", { length: 20 }).notNull().default("verified"),

    cliVersion: varchar("cli_version", { length: 20 }),
    submissionHash: varchar("submission_hash", { length: 64 }).unique(),

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

    date: date("date").notNull(),
    tokens: bigint("tokens", { mode: "number" }).notNull(),
    cost: decimal("cost", { precision: 10, scale: 4 }).notNull(),
    inputTokens: bigint("input_tokens", { mode: "number" }).notNull(),
    outputTokens: bigint("output_tokens", { mode: "number" }).notNull(),

    providerBreakdown: jsonb("provider_breakdown").$type<
      Record<string, number>
    >(),
    sourceBreakdown: jsonb("source_breakdown").$type<
      Record<
        string,
        | number
        | {
            tokens: number;
            cost: number;
            modelId: string;
            input: number;
            output: number;
            cacheRead: number;
            cacheWrite: number;
            messages: number;
          }
      >
    >(),
    modelBreakdown: jsonb("model_breakdown").$type<Record<string, number>>(),
  },
  (table) => [
    index("idx_daily_breakdown_submission_id").on(table.submissionId),
    index("idx_daily_breakdown_date").on(table.date),
    unique("daily_breakdown_submission_date_unique").on(
      table.submissionId,
      table.date
    ),
  ]
);

export const dailyBreakdownRelations = relations(dailyBreakdown, ({ one }) => ({
  submission: one(submissions, {
    fields: [dailyBreakdown.submissionId],
    references: [submissions.id],
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
export type DailyBreakdown = typeof dailyBreakdown.$inferSelect;
export type NewDailyBreakdown = typeof dailyBreakdown.$inferInsert;
