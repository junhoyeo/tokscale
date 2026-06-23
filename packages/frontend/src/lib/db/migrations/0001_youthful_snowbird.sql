-- Migration: Fix token totals that only counted input + output
-- Issue: sourceContributionToBreakdownData was calculating tokens = input + output
-- Correct: tokens = input + output + cacheRead + cacheWrite + reasoning
--
-- This migration:
-- 1. Updates sourceBreakdown JSONB tokens field for each source
-- 2. Recalculates daily_breakdown.tokens from corrected JSONB
-- 3. Recalculates submissions.totalTokens from daily_breakdown

-- Step 1: Fix sourceBreakdown.*.tokens inside JSONB
-- For each source, set tokens = input + output + cacheRead + cacheWrite
-- Note: reasoning is not stored in current schema, so we can only add what we have
UPDATE daily_breakdown
SET source_breakdown = (
  SELECT jsonb_object_agg(
    source_key,
    source_value || jsonb_build_object(
      'tokens', 
      COALESCE((source_value->>'input')::bigint, 0) + 
      COALESCE((source_value->>'output')::bigint, 0) + 
      COALESCE((source_value->>'cacheRead')::bigint, 0) + 
      COALESCE((source_value->>'cacheWrite')::bigint, 0)
    )
  )
  FROM jsonb_each(source_breakdown) AS t(source_key, source_value)
)
WHERE source_breakdown IS NOT NULL;

-- Step 2: Recalculate daily_breakdown.tokens from corrected sourceBreakdown
UPDATE daily_breakdown
SET tokens = (
  SELECT COALESCE(SUM(
    COALESCE((source_value->>'input')::bigint, 0) + 
    COALESCE((source_value->>'output')::bigint, 0) + 
    COALESCE((source_value->>'cacheRead')::bigint, 0) + 
    COALESCE((source_value->>'cacheWrite')::bigint, 0)
  ), 0)
  FROM jsonb_each(source_breakdown) AS t(source_key, source_value)
)
WHERE source_breakdown IS NOT NULL;

-- Step 3: Recalculate submissions.totalTokens from daily_breakdown
UPDATE submissions s
SET total_tokens = (
  SELECT COALESCE(SUM(db.tokens), 0)
  FROM daily_breakdown db
  WHERE db.submission_id = s.id
);
