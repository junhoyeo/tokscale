import { sql } from "drizzle-orm";
import { z } from "zod";
import { db } from "@/lib/db";

export const RATCHET_CENSUS_DEFAULT_CANDIDATE_LIMIT = 25;
export const RATCHET_CENSUS_MAX_CANDIDATE_LIMIT = 100;

const divergenceBandSchema = z.enum([
  "pending",
  "warming",
  "under",
  "clean",
  "mild",
  "clear",
  "severe",
]);

const tokenTextSchema = z.union([z.string(), z.number()]).transform(String);

const coverageSchema = z.object({
  totalUsers: z.number().int().nonnegative(),
  measuredUsers: z.number().int().nonnegative(),
  totalTokens: tokenTextSchema,
  measuredTokens: tokenTextSchema,
  userCoverage: z.number().min(0).max(1),
  tokenCoverage: z.number().min(0).max(1),
  pendingWorkItems: z.number().int().nonnegative(),
});

const segmentSchema = z.object({
  key: z.string(),
  expectedCells: z.number().int().nonnegative(),
  measuredCells: z.number().int().nonnegative(),
  cellCoverage: z.number().min(0).max(1),
});

const reportPayloadSchema = z.object({
  coverage: coverageSchema,
  divergenceBands: z.array(
    z.object({
      band: divergenceBandSchema,
      users: z.number().int().nonnegative(),
      tokens: tokenTextSchema,
    })
  ),
  observedCells: z.object({
    comparableCells: z.number().int().nonnegative(),
    under: z.number().int().nonnegative(),
    clean: z.number().int().nonnegative(),
    mild: z.number().int().nonnegative(),
    clear: z.number().int().nonnegative(),
    severe: z.number().int().nonnegative(),
    maxRatio: z.number().nullable(),
  }),
  segments: z.object({
    byOrigin: z.array(segmentSchema),
    byClient: z.array(segmentSchema),
    byCliVersion: z.array(
      z.object({
        cliVersion: z.string(),
        users: z.number().int().nonnegative(),
        measuredUsers: z.number().int().nonnegative(),
        totalTokens: tokenTextSchema,
        measuredTokens: tokenTextSchema,
      })
    ),
  }),
  candidates: z.array(
    z.object({
      username: z.string(),
      totalTokens: tokenTextSchema,
      highwaterTokens: tokenTextSchema,
      ratio: z.number().positive(),
      band: z.enum(["under", "mild", "clear", "severe"]),
      expectedCells: z.number().int().positive(),
      measuredCells: z.number().int().positive(),
      cliVersion: z.string(),
      deviceCount: z.number().int().nonnegative(),
    })
  ),
});

export type RatchetCensusReport = z.infer<typeof reportPayloadSchema> & {
  generatedAt: string;
};

interface CensusDbRow extends Record<string, unknown> {
  report: unknown;
}

export function normalizeRatchetCensusCandidateLimit(value: unknown): number {
  const parsed = typeof value === "string" && value.trim() !== "" ? Number(value) : NaN;
  if (!Number.isInteger(parsed) || parsed <= 0) {
    return RATCHET_CENSUS_DEFAULT_CANDIDATE_LIMIT;
  }
  return Math.min(parsed, RATCHET_CENSUS_MAX_CANDIDATE_LIMIT);
}

/**
 * Builds the Phase 1 reconciliation gate inputs from one PostgreSQL snapshot.
 *
 * Read-only by construction: this is a single SELECT with no writable CTEs.
 * A user is "measured" only after every month/client/origin cell visible in
 * their guarded daily rows has a matching per-device high-water row and no
 * durable census work remains. Missing cells therefore mean "warming", never
 * zero. Phase 4a comparisons likewise join only cells explicitly present in
 * both stores; an omitted latest observation is not treated as a deletion.
 *
 * The report deliberately stops at gate inputs. It does not authorize Phase 2
 * or mutate served totals; rollout thresholds remain an operator decision.
 */
export async function getRatchetCensusReport(params?: {
  candidateLimit?: number;
  now?: Date;
}): Promise<RatchetCensusReport> {
  const candidateLimit = Math.min(
    Math.max(params?.candidateLimit ?? RATCHET_CENSUS_DEFAULT_CANDIDATE_LIMIT, 1),
    RATCHET_CENSUS_MAX_CANDIDATE_LIMIT
  );

  const result = await db.execute<CensusDbRow>(sql`
    WITH expected_cells AS (
      SELECT DISTINCT
        s.user_id,
        db.submitted_device_id,
        source.key AS client,
        COALESCE(NULLIF(source.value #>> '{provenance,origin}', ''), 'cli') AS origin,
        to_char(db.date, 'YYYY-MM') AS bucket_key
      FROM daily_breakdown AS db
      JOIN submissions AS s ON s.id = db.submission_id
      CROSS JOIN LATERAL jsonb_each(
        COALESCE(db.source_breakdown, '{}'::jsonb)
      ) AS source(key, value)
    ),
    expected_status AS (
      SELECT
        e.user_id,
        e.client,
        e.origin,
        COUNT(*)::int AS expected_cells,
        COUNT(t.submitted_device_id)::int AS measured_cells
      FROM expected_cells AS e
      LEFT JOIN submitted_device_client_totals AS t
        ON t.submitted_device_id = e.submitted_device_id
       AND t.client = e.client
       AND t.origin = e.origin
       AND t.bucket_width = 'month'
       AND t.bucket_key = e.bucket_key
      GROUP BY e.user_id, e.client, e.origin
    ),
    coverage_by_user AS (
      SELECT
        user_id,
        SUM(expected_cells)::int AS expected_cells,
        SUM(measured_cells)::int AS measured_cells
      FROM expected_status
      GROUP BY user_id
    ),
    highwater_by_user AS (
      SELECT
        d.user_id,
        COUNT(*)::int AS bucket_count,
        COALESCE(SUM(t.tokens_highwater), 0) AS highwater_tokens
      FROM submitted_device_client_totals AS t
      JOIN submitted_devices AS d ON d.id = t.submitted_device_id
      WHERE t.bucket_width = 'month'
      GROUP BY d.user_id
    ),
    pending_by_user AS (
      SELECT s.user_id, COUNT(*)::int AS pending_work
      FROM ratchet_census_work AS w
      JOIN submissions AS s ON s.id = w.submission_id
      GROUP BY s.user_id
    ),
    devices_by_user AS (
      SELECT user_id, COUNT(*)::int AS device_count
      FROM submitted_devices
      GROUP BY user_id
    ),
    per_user AS (
      SELECT
        u.username,
        s.user_id,
        s.total_tokens::numeric AS total_tokens,
        COALESCE(NULLIF(s.cli_version, ''), 'unknown') AS cli_version,
        COALESCE(c.expected_cells, 0) AS expected_cells,
        COALESCE(c.measured_cells, 0) AS measured_cells,
        COALESCE(h.bucket_count, 0) AS bucket_count,
        COALESCE(h.highwater_tokens, 0)::numeric AS highwater_tokens,
        COALESCE(p.pending_work, 0) AS pending_work,
        COALESCE(d.device_count, 0) AS device_count,
        (
          COALESCE(c.expected_cells, 0) > 0
          AND c.measured_cells = c.expected_cells
          AND COALESCE(p.pending_work, 0) = 0
          AND COALESCE(h.highwater_tokens, 0) > 0
        ) AS fully_measured
      FROM submissions AS s
      JOIN users AS u ON u.id = s.user_id
      LEFT JOIN coverage_by_user AS c ON c.user_id = s.user_id
      LEFT JOIN highwater_by_user AS h ON h.user_id = s.user_id
      LEFT JOIN pending_by_user AS p ON p.user_id = s.user_id
      LEFT JOIN devices_by_user AS d ON d.user_id = s.user_id
    ),
    classified AS (
      SELECT
        p.*,
        CASE
          WHEN p.pending_work > 0 THEN 'pending'
          WHEN NOT p.fully_measured THEN 'warming'
          WHEN p.total_tokens / NULLIF(p.highwater_tokens, 0) < 0.95 THEN 'under'
          WHEN p.total_tokens / NULLIF(p.highwater_tokens, 0) <= 1.05 THEN 'clean'
          WHEN p.total_tokens / NULLIF(p.highwater_tokens, 0) <= 1.25 THEN 'mild'
          WHEN p.total_tokens / NULLIF(p.highwater_tokens, 0) <= 2.0 THEN 'clear'
          ELSE 'severe'
        END AS band,
        CASE WHEN p.fully_measured
          THEN p.total_tokens / NULLIF(p.highwater_tokens, 0)
          ELSE NULL
        END AS ratio
      FROM per_user AS p
    ),
    site AS (
      SELECT
        COUNT(*)::int AS total_users,
        COUNT(*) FILTER (WHERE fully_measured)::int AS measured_users,
        COALESCE(SUM(total_tokens), 0) AS total_tokens,
        COALESCE(SUM(total_tokens) FILTER (WHERE fully_measured), 0) AS measured_tokens
      FROM classified
    ),
    pending_site AS (
      SELECT COUNT(*)::int AS pending_work_items FROM ratchet_census_work
    ),
    divergence AS (
      SELECT
        band,
        COUNT(*)::int AS users,
        COALESCE(SUM(total_tokens), 0) AS tokens
      FROM classified
      GROUP BY band
    ),
    observed_cell_ratios AS (
      SELECT
        (
          COALESCE((db.source_breakdown -> r.client ->> 'tokens')::numeric, 0)
          / NULLIF(r.tokens::numeric, 0)
        ) AS ratio
      FROM daily_breakdown_reported AS r
      JOIN daily_breakdown AS db
        ON db.submitted_device_id = r.submitted_device_id
       AND db.date = r.date
      WHERE r.tokens > 0
        AND db.source_breakdown ? r.client
    ),
    observed_cells AS (
      SELECT
        COUNT(*)::int AS comparable_cells,
        COUNT(*) FILTER (WHERE ratio < 0.95)::int AS under_count,
        COUNT(*) FILTER (WHERE ratio >= 0.95 AND ratio <= 1.05)::int AS clean_count,
        COUNT(*) FILTER (WHERE ratio > 1.05 AND ratio <= 1.25)::int AS mild_count,
        COUNT(*) FILTER (WHERE ratio > 1.25 AND ratio <= 2.0)::int AS clear_count,
        COUNT(*) FILTER (WHERE ratio > 2.0)::int AS severe_count,
        MAX(ratio) AS max_ratio
      FROM observed_cell_ratios
      WHERE ratio IS NOT NULL
    ),
    origin_segments AS (
      SELECT
        origin AS key,
        SUM(expected_cells)::int AS expected_cells,
        SUM(measured_cells)::int AS measured_cells
      FROM expected_status
      GROUP BY origin
    ),
    client_segments AS (
      SELECT
        client AS key,
        SUM(expected_cells)::int AS expected_cells,
        SUM(measured_cells)::int AS measured_cells
      FROM expected_status
      GROUP BY client
    ),
    version_segments AS (
      SELECT
        cli_version,
        COUNT(*)::int AS users,
        COUNT(*) FILTER (WHERE fully_measured)::int AS measured_users,
        COALESCE(SUM(total_tokens), 0) AS total_tokens,
        COALESCE(SUM(total_tokens) FILTER (WHERE fully_measured), 0) AS measured_tokens
      FROM classified
      GROUP BY cli_version
    ),
    candidates AS (
      SELECT *
      FROM classified
      WHERE fully_measured
        AND band IN ('under', 'mild', 'clear', 'severe')
      ORDER BY ABS(ratio - 1) DESC, total_tokens DESC, username
      LIMIT ${candidateLimit}
    )
    SELECT jsonb_build_object(
      'coverage', jsonb_build_object(
        'totalUsers', site.total_users,
        'measuredUsers', site.measured_users,
        'totalTokens', site.total_tokens::text,
        'measuredTokens', site.measured_tokens::text,
        'userCoverage', COALESCE(
          ROUND(site.measured_users::numeric / NULLIF(site.total_users, 0), 6), 0
        ),
        'tokenCoverage', COALESCE(
          ROUND(site.measured_tokens / NULLIF(site.total_tokens, 0), 6), 0
        ),
        'pendingWorkItems', pending_site.pending_work_items
      ),
      'divergenceBands', COALESCE((
        SELECT jsonb_agg(jsonb_build_object(
          'band', band,
          'users', users,
          'tokens', tokens::text
        ) ORDER BY CASE band
          WHEN 'pending' THEN 0 WHEN 'warming' THEN 1 WHEN 'under' THEN 2
          WHEN 'clean' THEN 3 WHEN 'mild' THEN 4 WHEN 'clear' THEN 5 ELSE 6 END)
        FROM divergence
      ), '[]'::jsonb),
      'observedCells', jsonb_build_object(
        'comparableCells', observed_cells.comparable_cells,
        'under', observed_cells.under_count,
        'clean', observed_cells.clean_count,
        'mild', observed_cells.mild_count,
        'clear', observed_cells.clear_count,
        'severe', observed_cells.severe_count,
        'maxRatio', observed_cells.max_ratio
      ),
      'segments', jsonb_build_object(
        'byOrigin', COALESCE((
          SELECT jsonb_agg(jsonb_build_object(
            'key', key,
            'expectedCells', expected_cells,
            'measuredCells', measured_cells,
            'cellCoverage', COALESCE(
              ROUND(measured_cells::numeric / NULLIF(expected_cells, 0), 6), 0
            )
          ) ORDER BY key) FROM origin_segments
        ), '[]'::jsonb),
        'byClient', COALESCE((
          SELECT jsonb_agg(jsonb_build_object(
            'key', key,
            'expectedCells', expected_cells,
            'measuredCells', measured_cells,
            'cellCoverage', COALESCE(
              ROUND(measured_cells::numeric / NULLIF(expected_cells, 0), 6), 0
            )
          ) ORDER BY expected_cells DESC, key) FROM client_segments
        ), '[]'::jsonb),
        'byCliVersion', COALESCE((
          SELECT jsonb_agg(jsonb_build_object(
            'cliVersion', cli_version,
            'users', users,
            'measuredUsers', measured_users,
            'totalTokens', total_tokens::text,
            'measuredTokens', measured_tokens::text
          ) ORDER BY users DESC, cli_version) FROM version_segments
        ), '[]'::jsonb)
      ),
      'candidates', COALESCE((
        SELECT jsonb_agg(jsonb_build_object(
          'username', username,
          'totalTokens', total_tokens::text,
          'highwaterTokens', highwater_tokens::text,
          'ratio', ratio,
          'band', band,
          'expectedCells', expected_cells,
          'measuredCells', measured_cells,
          'cliVersion', cli_version,
          'deviceCount', device_count
        ) ORDER BY ABS(ratio - 1) DESC, total_tokens DESC, username)
        FROM candidates
      ), '[]'::jsonb)
    ) AS report
    FROM site
    CROSS JOIN pending_site
    CROSS JOIN observed_cells
  `);

  const rows = result as unknown as CensusDbRow[];
  const parsed = reportPayloadSchema.parse(rows[0]?.report);
  return {
    ...parsed,
    generatedAt: (params?.now ?? new Date()).toISOString(),
  };
}
