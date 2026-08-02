import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import postgres from "postgres";

const databaseUrl = process.env.DATABASE_URL;

if (!databaseUrl) {
  throw new Error("DATABASE_URL is required");
}

const sql = postgres(databaseUrl, { max: 1 });

function expect(name: string, condition: boolean, detail?: string) {
  if (!condition) {
    throw new Error(`${name} failed${detail ? `: ${detail}` : ""}`);
  }

  console.log(`ok - ${name}`);
}

const journalPath = resolve(
  import.meta.dir,
  "../src/lib/db/migrations/meta/_journal.json"
);
const migrationJournal = JSON.parse(readFileSync(journalPath, "utf8")) as {
  entries: { idx: number; tag: string; when: number }[];
};

const idxValues = migrationJournal.entries.map((entry) => entry.idx);
const idxSet = new Set(idxValues);
if (idxSet.size !== idxValues.length) {
  const seen = new Set<number>();
  const dupes = idxValues.filter((idx) => {
    if (seen.has(idx)) return true;
    seen.add(idx);
    return false;
  });
  throw new Error(
    `_journal.json contains duplicate idx values: ${Array.from(new Set(dupes)).join(", ")}. ` +
      `Re-run drizzle-kit generate on the conflicting branch.`
  );
}

const sortedIdx = [...idxValues].sort((a, b) => a - b);
for (let i = 0; i < sortedIdx.length; i++) {
  if (sortedIdx[i] !== i) {
    throw new Error(
      `_journal.json has a gap or non-contiguous idx sequence at position ${i}: ` +
        `expected ${i}, got ${sortedIdx[i]}. Sequence must be 0..N-1.`
    );
  }
}
console.log(`ok - migration journal idx sequence is contiguous (0..${sortedIdx.length - 1})`);

// Drizzle replays by timestamp, not by idx: it fetches the last applied
// migration's created_at once and applies only entries whose `when` exceeds
// it. An entry added with a `when` earlier than an already-applied migration
// is therefore silently skipped on prod, while a fresh CI database (no prior
// migrations) still applies everything — so only a static journal check can
// catch it. Require array order, idx order, tag prefix, and `when` order to
// all agree.
for (let i = 0; i < migrationJournal.entries.length; i++) {
  const entry = migrationJournal.entries[i];
  if (entry.idx !== i) {
    throw new Error(
      `_journal.json entries must be listed in idx order: position ${i} has idx ${entry.idx}.`
    );
  }
  const tagPrefix = Number.parseInt(entry.tag.split("_")[0], 10);
  if (tagPrefix !== entry.idx) {
    throw new Error(
      `_journal.json entry idx ${entry.idx} does not match its tag prefix (${entry.tag}). ` +
        `Rename the migration so the file number and idx agree.`
    );
  }
  if (i > 0 && entry.when <= migrationJournal.entries[i - 1].when) {
    throw new Error(
      `_journal.json entry ${entry.tag} (when ${entry.when}) is not strictly newer than ` +
        `${migrationJournal.entries[i - 1].tag} (when ${migrationJournal.entries[i - 1].when}). ` +
        `Drizzle would silently skip it on databases that already applied the newer entry. ` +
        `Append new migrations at the end with a fresh timestamp (re-run drizzle-kit generate); ` +
        `never hand-edit or reorder existing entries.`
    );
  }
}
console.log(
  "ok - migration journal `when` timestamps strictly increase in idx order"
);

const migrationsDir = resolve(import.meta.dir, "../src/lib/db/migrations");

// The reverse of the missing-file check below: a .sql file on disk with no
// journal entry is never applied anywhere — drizzle iterates only journal
// entries — and the applied-count assertion still passes because it too
// counts journal entries. Typically the residue of a botched conflict
// resolution that kept the file but dropped the entry.
const journalTags = new Set(migrationJournal.entries.map((entry) => entry.tag));
const orphanSqlFiles = readdirSync(migrationsDir)
  .filter((name) => name.endsWith(".sql"))
  .filter((name) => !journalTags.has(name.slice(0, -".sql".length)))
  .sort();
if (orphanSqlFiles.length > 0) {
  throw new Error(
    `migration files exist on disk with no _journal.json entry (drizzle would never apply them): ` +
      `${orphanSqlFiles.join(", ")}. Re-run drizzle-kit generate or restore the journal entry.`
  );
}
console.log("ok - every migration .sql file has a journal entry");

// drizzle-kit generates against the lexically newest meta/*_snapshot.json.
// If the newest snapshot lags the journal tail (a migration merged without
// its snapshot), the next generate re-emits DDL that is already applied —
// e.g. a duplicate ADD COLUMN that fails on apply. Historical snapshot gaps
// are fine (this repo has them); only the tail must be current.
const snapshotIndexes = readdirSync(resolve(migrationsDir, "meta"))
  .map((name) => /^(\d{4})_snapshot\.json$/.exec(name)?.[1])
  .filter((idx): idx is string => idx !== undefined)
  .map((idx) => Number.parseInt(idx, 10));
const journalTailIdx =
  migrationJournal.entries[migrationJournal.entries.length - 1]?.idx;
if (journalTailIdx !== undefined) {
  const newestSnapshotIdx =
    snapshotIndexes.length > 0 ? Math.max(...snapshotIndexes) : undefined;
  if (newestSnapshotIdx !== journalTailIdx) {
    throw new Error(
      `newest migration snapshot (${
        newestSnapshotIdx === undefined
          ? "none found"
          : `${String(newestSnapshotIdx).padStart(4, "0")}_snapshot.json`
      }) does not match the journal tail (idx ${journalTailIdx}). ` +
        `drizzle-kit generate would re-emit already-applied DDL against the stale baseline. ` +
        `Commit the meta/${String(journalTailIdx).padStart(4, "0")}_snapshot.json produced by drizzle-kit generate.`
    );
  }
}
console.log("ok - newest migration snapshot matches the journal tail");

for (const entry of migrationJournal.entries) {
  const sqlPath = resolve(migrationsDir, `${entry.tag}.sql`);
  let sqlBody: string;
  try {
    sqlBody = readFileSync(sqlPath, "utf8");
  } catch (err) {
    // A missing file referenced by the journal is a real bug (someone
    // deleted a migration). Anything else (EACCES, etc.) is also unexpected
    // — surface it loudly rather than silently skip.
    if ((err as NodeJS.ErrnoException).code === "ENOENT") {
      throw new Error(
        `_journal.json references ${entry.tag}.sql but the file is missing on disk`
      );
    }
    throw err;
  }
  const stripped = sqlBody
    .replace(/--[^\n]*/g, "")
    .replace(/\/\*[\s\S]*?\*\//g, "");
  const tables = new Set<string>();
  // Capture the table name from `ON [ONLY] [<schema>.]<table>`. The
  // optional non-capturing schema prefix lets us count schema-qualified
  // targets (`ON public.users`) the same as bare names (`ON users`).
  const re = /\bCREATE\s+(?:UNIQUE\s+)?INDEX\b[\s\S]*?\bON\s+(?:ONLY\s+)?(?:(?:"[^"]+"|`[^`]+`|\w+)\.)?(?:"([^"]+)"|`([^`]+)`|(\w+))/gi;
  let match: RegExpExecArray | null;
  while ((match = re.exec(stripped)) !== null) {
    const name = match[1] ?? match[2] ?? match[3];
    if (name) tables.add(name.toLowerCase());
  }
  if (tables.size > 1) {
    console.warn(
      `warn - migration ${entry.tag}.sql creates indexes on ${tables.size} tables ` +
        `(${Array.from(tables).join(", ")}). Drizzle wraps each migration in one transaction; ` +
        `multi-table index builds hold ACCESS EXCLUSIVE across all of them. Consider splitting.`
    );
  }
}

try {
  const [{ count: appliedMigrationCount }] = await sql<
    { count: number }[]
  >`SELECT count(*)::int AS count FROM "drizzle"."__drizzle_migrations"`;

  expect(
    "all migration journal entries were applied",
    appliedMigrationCount === migrationJournal.entries.length,
    `expected ${migrationJournal.entries.length}, got ${appliedMigrationCount}`
  );

  const requiredTables = await sql<{ table_name: string }[]>`
    SELECT table_name
    FROM information_schema.tables
    WHERE table_schema = 'public'
      AND table_name IN (
        'api_tokens',
        'daily_breakdown',
        'device_codes',
        'group_invites',
        'group_members',
        'groups',
        'sessions',
        'submissions',
        'submitted_devices',
        'users'
      )
  `;
  const tableNames = new Set(requiredTables.map((row) => row.table_name));
  const missingTables = [
    "api_tokens",
    "daily_breakdown",
    "device_codes",
    "group_invites",
    "group_members",
    "groups",
    "sessions",
    "submissions",
    "submitted_devices",
    "users",
  ].filter((tableName) => !tableNames.has(tableName));

  expect(
    "required public tables exist",
    missingTables.length === 0,
    missingTables.join(", ")
  );

  const columnRows = await sql<
    {
      table_name: string;
      column_name: string;
      is_nullable: string;
      column_default: string | null;
    }[]
  >`
    SELECT table_name, column_name, is_nullable, column_default
    FROM information_schema.columns
    WHERE table_schema = 'public'
      AND (
        (table_name = 'submissions' AND column_name IN ('reasoning_tokens', 'schema_version', 'submit_count'))
        OR (table_name = 'daily_breakdown' AND column_name IN ('submitted_device_id', 'active_time_ms'))
      )
  `;
  const columns = new Map(
    columnRows.map((row) => [`${row.table_name}.${row.column_name}`, row])
  );

  expect(
    "submit_count is present with a default",
    columns.get("submissions.submit_count")?.column_default === "1"
  );
  expect(
    "submitted_device_id is required",
    columns.get("daily_breakdown.submitted_device_id")?.is_nullable === "NO"
  );
  expect(
    "time and schema columns are present",
    [
      "submissions.reasoning_tokens",
      "submissions.schema_version",
      "daily_breakdown.active_time_ms",
    ].every((columnName) => columns.has(columnName))
  );

  const removedColumns = await sql<{ count: number }[]>`
    SELECT count(*)::int AS count
    FROM information_schema.columns
    WHERE table_schema = 'public'
      AND (
        (table_name = 'users' AND column_name = 'is_admin')
        OR (table_name = 'submissions' AND column_name = 'status')
        OR (table_name = 'daily_breakdown' AND column_name IN ('provider_breakdown', 'model_breakdown'))
      )
  `;

  expect("removed columns stay removed", removedColumns[0].count === 0);

  const indexRows = await sql<{ indexname: string; indexdef: string }[]>`
    SELECT indexname, indexdef
    FROM pg_indexes
    WHERE schemaname = 'public'
      AND indexname IN (
        'idx_device_codes_user_id',
        'idx_group_invites_invited_by',
        'idx_group_members_invited_by',
        'idx_submissions_leaderboard',
        'users_username_lower_unique'
      )
  `;
  const indexes = new Map(indexRows.map((row) => [row.indexname, row.indexdef]));

  expect(
    "required indexes exist",
    [
      "idx_device_codes_user_id",
      "idx_group_invites_invited_by",
      "idx_group_members_invited_by",
      "idx_submissions_leaderboard",
      "users_username_lower_unique",
    ].every((indexName) => indexes.has(indexName))
  );
  expect(
    "case-insensitive username index is unique",
    indexes.get("users_username_lower_unique")?.includes("UNIQUE INDEX") === true &&
      indexes.get("users_username_lower_unique")?.includes("lower((username)::text)") ===
        true
  );

  const extensionRows = await sql<{ count: number }[]>`
    SELECT count(*)::int AS count
    FROM pg_extension
    WHERE extname = 'pgcrypto'
  `;

  expect("pgcrypto extension is available", extensionRows[0].count === 1);

  await sql`BEGIN`;
  try {
    const [user] = await sql<{ id: string }[]>`
      INSERT INTO "users" ("github_id", "username")
      VALUES (1001, 'ci_migration_replay')
      RETURNING "id"
    `;
    const [submission] = await sql<{ id: string }[]>`
      INSERT INTO "submissions" (
        "user_id",
        "total_tokens",
        "total_cost",
        "input_tokens",
        "output_tokens",
        "date_start",
        "date_end",
        "sources_used",
        "models_used"
      )
      VALUES (
        ${user.id},
        42,
        0.4200,
        20,
        22,
        '2026-05-25',
        '2026-05-25',
        ARRAY['codex'],
        ARRAY['gpt-5']
      )
      RETURNING "id"
    `;
    const [device] = await sql<{ id: string }[]>`
      INSERT INTO "submitted_devices" ("user_id", "device_key", "display_name")
      VALUES (${user.id}, 'ci-device', 'CI device')
      RETURNING "id"
    `;
    await sql`
      INSERT INTO "daily_breakdown" (
        "submission_id",
        "submitted_device_id",
        "date",
        "tokens",
        "cost",
        "input_tokens",
        "output_tokens"
      )
      VALUES (${submission.id}, ${device.id}, '2026-05-25', 42, 0.4200, 20, 22)
    `;
    await sql`
      INSERT INTO "groups" ("name", "slug", "created_by")
      VALUES ('CI Group', 'ci-group', ${user.id})
    `;
  } finally {
    await sql`ROLLBACK`;
  }

  expect("schema accepts representative inserts", true);
} finally {
  await sql.end();
}
