-- Phase 1 of backfill-provenance persistence (issue #888): submissions gains
-- a sticky has_backfill flag set once any accepted submission carries a
-- submission-level provenance.origin = 'backfill' tag (from `tokscale
-- import`). It is display-only in this phase (profile/embed badge); ranking
-- and leaderboard queries deliberately do not read it yet.
--
-- DEFAULT false NOT NULL backfills every existing row to false, which is
-- correct: no historical submission could have carried the tag because the
-- submit route dropped it before this migration existed.
ALTER TABLE "submissions" ADD COLUMN "has_backfill" boolean DEFAULT false NOT NULL;
