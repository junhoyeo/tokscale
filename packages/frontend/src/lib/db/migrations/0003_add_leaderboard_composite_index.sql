CREATE INDEX IF NOT EXISTS "idx_submissions_leaderboard" ON "submissions" ("user_id", "total_tokens", "total_cost", "created_at");
