/**
 * Shared contract for the Humans / Models / Agents explore surfaces.
 *
 * UI code imports only from "@/lib/explore". The implementation behind these
 * functions can change (fixtures -> database rollups) without touching pages.
 *
 * Rules every implementation must keep:
 * - Accounts with users.leaderboard_hidden = true never count anywhere.
 * - "activeUsers" = distinct accounts that submitted usage of the model/agent in the period.
 * - Cost is always an estimate from public list prices; UI labels it "Est. cost".
 * - Model keys are canonical (lowercase, date suffix stripped, see canonical_model_id in tokscale-core).
 * - Unknown/unidentified models are counted in coverage, never silently dropped.
 */

export type ExplorePeriod = "7d" | "30d" | "all";
export type ExploreSort = "users" | "tokens";

export interface TokenMix {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
}

export interface ModelRef {
  /** Canonical model key, URL-safe, e.g. "claude-opus-5". */
  key: string;
  /** Display name, e.g. "Claude Opus 5". */
  name: string;
  /** Lab/company display name, or null when unknown. */
  lab: string | null;
  /** Lab/company slug for /capital/companies/[slug], or null. */
  labSlug: string | null;
  /** Model family mark (e.g. "/assets/v2/models/claude.svg"), or null when the family is unknown. */
  logoUrl: string | null;
}

export interface AgentRef {
  /** Client id as submitted, e.g. "claude", "codex", "opencode". */
  key: string;
  /** Display name, e.g. "Claude Code". */
  name: string;
  logoUrl: string | null;
}

export interface Coverage {
  period: ExplorePeriod;
  /** Inclusive ISO dates (UTC). For "all" the first date with data. */
  start: string;
  end: string;
  /** Distinct visible accounts with any usage in the period. */
  accounts: number;
  /** Share of tokens whose model was identified (0..1). */
  modelIdentifiedShare: number;
  /** ISO timestamp of the newest submission included. */
  lastDataAt: string | null;
}

export interface ModelUsageRow extends ModelRef {
  rank: number;
  activeUsers: number;
  /** Same-length previous window; null for "all". */
  previousActiveUsers: number | null;
  /** activeUsers / coverage.accounts (0..1). Sums across models can exceed 1. */
  adoption: number;
  tokens: number;
  estimatedCost: number;
  /** True when the model had no usage before the current window. */
  isNew: boolean;
  /** Up to 3 agents that carried most of this model's tokens. */
  topAgents: Array<AgentRef & { share: number }>;
}

export interface AgentUsageRow extends AgentRef {
  rank: number;
  activeUsers: number;
  previousActiveUsers: number | null;
  adoption: number;
  tokens: number;
  estimatedCost: number;
  isNew: boolean;
  /** Up to 3 models used most through this agent. */
  topModels: Array<ModelRef & { share: number }>;
}

export interface HumanRow {
  rank: number;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  tokens: number;
  estimatedCost: number;
  activeDays: number;
  /** Up to 3, by tokens in the period. */
  topModels: ModelRef[];
  /** Up to 3, by tokens in the period. */
  topAgents: AgentRef[];
}

export interface Ranked<T> {
  rows: T[];
  coverage: Coverage;
}

export interface ModelDetail {
  model: ModelRef & { aliases: string[] };
  coverage: Coverage;
  summary: {
    rank: number | null;
    activeUsers: number;
    previousActiveUsers: number | null;
    adoption: number;
    tokens: number;
    estimatedCost: number;
    tokenMix: TokenMix;
    firstSeen: string | null;
  };
  /** Daily points across the period (UTC dates, zero-filled). */
  trend: Array<{ date: string; activeUsers: number; tokens: number }>;
  agents: Array<AgentRef & { activeUsers: number; share: number }>;
  /** Public profiles that used the model most in the period. */
  humans: HumanRow[];
  /** Other models from the same lab, by active users. */
  related: ModelUsageRow[];
}

export interface CommunitySnapshot {
  humans: HumanRow[];
  models: ModelUsageRow[];
  agents: AgentUsageRow[];
  coverage: Coverage;
}

export interface LeaderboardQuery {
  period?: ExplorePeriod;
  sort?: ExploreSort;
  limit?: number;
  offset?: number;
}
