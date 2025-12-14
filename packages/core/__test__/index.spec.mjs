import test from "ava";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync, rmSync, existsSync } from "node:fs";

// ESM dirname equivalent
const __dirname = dirname(fileURLToPath(import.meta.url));

// Import native module
let nativeModule;
try {
  nativeModule = await import("../index.js");
} catch (e) {
  console.error("Failed to load native module:", e.message);
}

// Skip all tests if native module is not available
const testFn = nativeModule ? test : test.skip;

testFn("version returns semver string", (t) => {
  const v = nativeModule.version();
  t.regex(v, /^\d+\.\d+\.\d+$/);
});

testFn("healthCheck returns expected message", (t) => {
  t.is(nativeModule.healthCheck(), "tokscale-core is healthy!");
});

testFn("scanSessions with empty directory returns zeros", (t) => {
  const tmpDir = join(__dirname, "tmp-scan-test-" + Date.now());
  mkdirSync(tmpDir, { recursive: true });

  try {
    const stats = nativeModule.scanSessions(tmpDir, ["opencode", "claude"]);
    t.is(stats.totalFiles, 0);
    t.is(stats.opencodeFiles, 0);
    t.is(stats.claudeFiles, 0);
    t.is(stats.codexFiles, 0);
    t.is(stats.geminiFiles, 0);
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }
});

testFn("scanSessions finds OpenCode files in fixtures", (t) => {
  const fixturesDir = join(__dirname, "fixtures");
  
  if (!existsSync(fixturesDir)) {
    t.pass("Fixtures directory not found, skipping");
    return;
  }

  const stats = nativeModule.scanSessions(fixturesDir, ["opencode"]);
  t.true(stats.opencodeFiles >= 1, "Should find at least 1 OpenCode file");
});

testFn("scanSessions finds Claude files in fixtures", (t) => {
  const fixturesDir = join(__dirname, "fixtures");
  
  if (!existsSync(fixturesDir)) {
    t.pass("Fixtures directory not found, skipping");
    return;
  }

  const stats = nativeModule.scanSessions(fixturesDir, ["claude"]);
  t.true(stats.claudeFiles >= 1, "Should find at least 1 Claude file");
});

testFn("generateGraph returns valid structure", (t) => {
  const tmpDir = join(__dirname, "tmp-graph-test-" + Date.now());
  setupMockOpenCodeSession(tmpDir);

  try {
    const result = nativeModule.generateGraph({
      homeDir: tmpDir,
      sources: ["opencode"],
    });

    // Verify structure
    t.truthy(result.meta, "Should have meta");
    t.truthy(result.summary, "Should have summary");
    t.true(Array.isArray(result.years), "years should be array");
    t.true(Array.isArray(result.contributions), "contributions should be array");

    // Verify meta fields
    t.truthy(result.meta.generatedAt);
    t.truthy(result.meta.version);
    t.true(result.meta.processingTimeMs >= 0);
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }
});

testFn("generateGraph with year filter", (t) => {
  const tmpDir = join(__dirname, "tmp-year-test-" + Date.now());
  setupMockOpenCodeSession(tmpDir);

  try {
    const result = nativeModule.generateGraph({
      homeDir: tmpDir,
      sources: ["opencode"],
      year: "2024",
    });

    // All contributions should be from 2024
    for (const contrib of result.contributions) {
      t.true(contrib.date.startsWith("2024-"), `Date ${contrib.date} should be in 2024`);
    }
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }
});

testFn("generateGraph with date range filter", (t) => {
  const tmpDir = join(__dirname, "tmp-range-test-" + Date.now());
  setupMockOpenCodeSession(tmpDir);

  try {
    const result = nativeModule.generateGraph({
      homeDir: tmpDir,
      sources: ["opencode"],
      since: "2024-12-01",
      until: "2024-12-31",
    });

    // All contributions should be in range
    for (const contrib of result.contributions) {
      t.true(contrib.date >= "2024-12-01", `Date ${contrib.date} should be >= 2024-12-01`);
      t.true(contrib.date <= "2024-12-31", `Date ${contrib.date} should be <= 2024-12-31`);
    }
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }
});

testFn("generateGraph calculates token breakdown", (t) => {
  const tmpDir = join(__dirname, "tmp-tokens-test-" + Date.now());
  setupMockOpenCodeSession(tmpDir);

  try {
    const result = nativeModule.generateGraph({
      homeDir: tmpDir,
      sources: ["opencode"],
    });

    if (result.contributions.length > 0) {
      const contrib = result.contributions[0];
      t.truthy(contrib.tokenBreakdown, "Should have token breakdown");
      t.true(typeof contrib.tokenBreakdown.input === "number");
      t.true(typeof contrib.tokenBreakdown.output === "number");
      t.true(typeof contrib.tokenBreakdown.cacheRead === "number");
      t.true(typeof contrib.tokenBreakdown.cacheWrite === "number");
    }
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }
});

testFn("generateGraph handles empty directory gracefully", (t) => {
  const tmpDir = join(__dirname, "tmp-empty-test-" + Date.now());
  mkdirSync(tmpDir, { recursive: true });

  try {
    const result = nativeModule.generateGraph({
      homeDir: tmpDir,
      sources: ["opencode", "claude"],
    });

    t.truthy(result);
    t.is(result.contributions.length, 0);
    t.is(result.summary.totalTokens, 0);
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }
});

testFn("generateGraph with fixtures directory", (t) => {
  const fixturesDir = join(__dirname, "fixtures");
  
  if (!existsSync(fixturesDir)) {
    t.pass("Fixtures directory not found, skipping");
    return;
  }

  const result = nativeModule.generateGraph({
    homeDir: fixturesDir,
    sources: ["opencode", "claude"],
  });

  t.truthy(result);
  t.true(result.summary.totalTokens > 0, "Should have parsed some tokens");
});

// Helper function to create mock OpenCode session
function setupMockOpenCodeSession(baseDir) {
  const sessionDir = join(
    baseDir,
    ".local/share/opencode/storage/message/test-project"
  );
  mkdirSync(sessionDir, { recursive: true });

  const mockMessage = {
    id: "msg_mock001",
    sessionID: "ses_mock001",
    role: "assistant",
    time: {
      created: 1733011200000, // 2024-12-01
      completed: 1733011260000,
    },
    modelID: "claude-3-5-sonnet-20241022",
    providerID: "anthropic",
    tokens: {
      input: 1000,
      output: 500,
      reasoning: 100,
      cache: {
        read: 200,
        write: 50,
      },
    },
  };

  writeFileSync(
    join(sessionDir, "msg_mock001.json"),
    JSON.stringify(mockMessage, null, 2)
  );
}
