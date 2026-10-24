import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createGitHubClient, GitHubApiError } from "./github.mjs";
import { runCla, loadPullRequest, SIGN_TEXT, STATUS_CONTEXT, parseStorage } from "./check.mjs";
import { initializeStorage } from "./initialize.mjs";

const REPO = "junhoyeo/tokscale";
const SHA = "a".repeat(40);
const owner = { login: "junhoyeo", databaseId: 1, __typename: "User" };
const alice = { login: "alice", databaseId: 2, __typename: "User" };
const bob = { login: "bob", databaseId: 3, __typename: "User" };
const bot = { login: "github-actions[bot]", databaseId: 41898282, __typename: "User" };
const asRestUser = (user) => ({ login: user.login, id: user.databaseId,
  type: user.login.endsWith("[bot]") ? "Bot" : "User" });
const signature = (user = alice, body = SIGN_TEXT, id = 31) => ({ id, body,
  user: asRestUser(user), created_at: "2026-10-22T00:00:00Z" });
const commit = (authors = [owner], i = 1) => ({ id: `commit-${i}`, oid: i.toString(16).padStart(40, "0"), authors });
const response = (data, status = 200, headers) => new Response(JSON.stringify(data), { status, headers });
const connection = (nodes, cursor, size = 100) => {
  const start = cursor ? Number(cursor) : 0;
  return { nodes: nodes.slice(start, start + size), totalCount: nodes.length,
    pageInfo: { hasNextPage: start + size < nodes.length, endCursor: String(start + size) } };
};

function fixture(options = {}) {
  const commits = options.commits ?? [commit([alice])];
  const files = options.files ?? [{ filename: "packages/frontend/src/app/page.tsx" }];
  const comments = options.comments ?? [];
  let doc = options.document ?? { signedContributors: [] };
  let conflicts = 0;
  const pr = { number: 42, state: "open", head: { sha: SHA }, base: { sha: "b".repeat(40), repo: { full_name: REPO, id: 101 } },
    user: asRestUser(options.author ?? owner), commits: commits.length, changed_files: files.length, ...options.pr };
  const event = { action: "opened", repository: { full_name: REPO, id: 101, default_branch: "main" }, pull_request: { number: 42 }, ...options.event };
  const calls = [];
  let prReads = 0;
  const fetchMock = async (url, init) => {
    const path = url.pathname;
    const body = init.body ? JSON.parse(init.body) : undefined;
    calls.push({ path, method: init.method, body, url: url.href, init });
    assert.equal(init.redirect, "error");
    assert.ok(init.signal instanceof AbortSignal);
    if (init.method === "POST" && path === "/graphql") {
      const variables = body.variables;
      if (variables.id) {
        const item = commits.find((item) => item.id === variables.id);
        return response({ data: { node: { authors: connection(item.authors.map((user) => ({ user })), variables.cursor) } } });
      }
      const page = connection(commits.map((item) => ({ commit: { ...item,
        authors: connection(item.authors.map((user) => ({ user })), null) } })), variables.cursor);
      if (options.truncateCommits) page.pageInfo.hasNextPage = false;
      if (options.repeatCursor && variables.cursor) page.pageInfo = { hasNextPage: true, endCursor: variables.cursor };
      return response({ data: { repository: { pullRequest: { headRefOid: options.graphHead ?? SHA, commits: page } } } });
    }
    if (init.method === "GET" && path.endsWith("/pulls/42")) {
      prReads++;
      return response(options.changedPr && prReads > 1 ? { ...pr, ...options.changedPr } : pr);
    }
    if (init.method === "GET" && path.includes("/contents/")) {
      if (options.storageError) return response({ message: "error" }, options.storageError);
      return response({ type: "file", encoding: "base64", sha: "c".repeat(40),
        content: Buffer.from(JSON.stringify(doc)).toString("base64") });
    }
    if (init.method === "GET" && (path.endsWith("/files") || path.endsWith("/comments") || path.endsWith("/pulls"))) {
      const items = path.endsWith("/pulls") ? (options.openPrs ?? [pr]) : path.endsWith("/files") ? files : comments;
      if (path.endsWith("/pulls")) assert.equal(url.searchParams.get("state"), "open");
      const page = Number(url.searchParams.get("page") ?? 1);
      const pageSize = 100;
      const nextUrl = new URL(url);
      nextUrl.searchParams.set("page", String(page + 1));
      if (options.canonicalLinks) nextUrl.pathname = path.replace(`/repos/${REPO}`, "/repositories/101");
      const headers = page * pageSize < items.length
        ? { link: `<${nextUrl.href}>; rel="next"` } : undefined;
      return response(items.slice((page - 1) * pageSize, page * pageSize), 200, headers);
    }
    if (["POST", "PATCH", "PUT"].includes(init.method)) {
      if (init.method === "PUT" && (options.writeConflict || (options.conflictOnce && conflicts === 0))) {
        conflicts++;
        doc = options.concurrentDocument ?? doc;
        return response({ message: "conflict" }, 409);
      }
      return response({ id: 1000 });
    }
    throw new Error(`Unexpected request ${init.method} ${url}`);
  };
  const api = createGitHubClient("test-token", fetchMock);
  return { calls, api, event, run: (extra = {}) => runCla({ api, repository: REPO, eventName: "pull_request_target", event, ...extra }) };
}
const writes = (f) => f.calls.filter((call) => ["POST", "PUT", "PATCH"].includes(call.method) && call.path !== "/graphql");
const signaturesWritten = (f) => {
  const write = f.calls.find((call) => call.method === "PUT");
  return write ? JSON.parse(Buffer.from(write.body.content, "base64").toString("utf8")).signedContributors : [];
};
const statusStates = (f) => f.calls.filter((call) => call.path.includes("/statuses/")).map((call) => {
  assert.equal(call.body.context, STATUS_CONTEXT);
  return call.body.state;
});

test("the exact full comment signs once and preserves consent evidence", async () => {
  const f = fixture({ comments: [signature(), signature(alice, SIGN_TEXT, 32)] });
  assert.equal((await f.run()).state, "success");
  assert.deepEqual(statusStates(f), ["pending", "success"]);
  const saved = signaturesWritten(f);
  assert.equal(saved.length, 1);
  assert.equal(saved[0].body, SIGN_TEXT);
  assert.equal(saved[0].comment_id, 31);
  assert.equal(saved[0].commentUrl, `https://github.com/${REPO}/pull/42#issuecomment-31`);
  const put = f.calls.find((call) => call.method === "PUT");
  assert.equal(put.body.branch, "cla-signatures");
  assert.equal(put.body.sha, "c".repeat(40));
});

for (const body of [
  `I do NOT agree to this CLA. I will not post "${SIGN_TEXT}".`,
  `> ${SIGN_TEXT}`, `"${SIGN_TEXT}"`, `\`${SIGN_TEXT}\``,
  `${SIGN_TEXT}\nI refuse.`, `${SIGN_TEXT} `, `\n${SIGN_TEXT}`, SIGN_TEXT.toLowerCase(),
  `$(touch /tmp/cla-should-not-exist) ${SIGN_TEXT}`,
]) {
  test(`reject non-exact consent: ${JSON.stringify(body)}`, async () => {
    const f = fixture({ comments: [signature(alice, body)] });
    assert.equal((await f.run()).state, "failure");
    assert.equal(signaturesWritten(f).length, 0);
  });
}

for (const count of [101, 251]) {
  test(`checks the new unsigned author in commit ${count}`, async () => {
    const f = fixture({ commits: [...Array.from({ length: count - 1 }, (_, i) => commit([owner], i + 1)), commit([bob], count)] });
    const result = await f.run();
    assert.equal(result.state, "failure");
    assert.deepEqual(result.unsigned, [{ id: 3, name: "bob" }]);
    assert.equal(f.calls.filter((call) => call.path === "/graphql").length, Math.ceil(count / 100));
  });
}
for (const index of [31, 131]) {
  test(`reads a signature at comment ${index}`, async () => {
    const comments = Array.from({ length: index - 1 }, (_, i) => signature(alice, `discussion ${i}`, i + 1));
    const f = fixture({ comments: [...comments, signature(alice, SIGN_TEXT, index)] });
    assert.equal((await f.run()).state, "success");
    assert.equal(signaturesWritten(f)[0].comment_id, index);
    assert.equal(f.calls.filter((call) => call.method === "GET" && call.path.endsWith("/comments")).length, Math.ceil(index / 100));
  });
}

test("paginates coauthors and requires a late coauthor independently", async () => {
  const f = fixture({ commits: [commit([...Array.from({ length: 100 }, () => owner), bob])] });
  assert.deepEqual((await f.run()).unsigned, [{ id: 3, name: "bob" }]);
  assert.equal(f.calls.filter((call) => call.body?.variables?.id).length, 1);
});
test("unknown authors fail closed instead of borrowing the committer's identity", async () => {
  const f = fixture({ commits: [commit([null])] });
  const result = await f.run();
  assert.equal(result.state, "failure");
  assert.equal(result.unknown.length, 1);
  assert.ok(writes(f).some((call) => call.body?.body?.includes("not linked to a GitHub account")));
});
test("the pull request author must sign even when commits belong to the owner", async () => {
  const f = fixture({ author: bob, commits: [commit([owner])] });
  assert.deepEqual((await f.run()).unsigned, [{ id: 3, name: "bob" }]);
});
test("owner and GitHub-reserved bot accounts are exempt without storage", async () => {
  const f = fixture({ author: bot, commits: [commit([owner, bot])], storageError: 404 });
  assert.equal((await f.run()).state, "success");
  assert.ok(!f.calls.some((call) => call.path.includes("/contents/")));
});
test("a signed GitHub id remains signed after a login rename", async () => {
  const f = fixture({ document: { signedContributors: [{ id: alice.databaseId, name: "previous-name" }] } });
  assert.equal((await f.run()).state, "success");
  assert.equal(signaturesWritten(f).length, 0);
});
test("an account reusing a signed login does not inherit its signature", async () => {
  const f = fixture({ document: { signedContributors: [{ id: 999, name: alice.login }] } });
  assert.equal((await f.run()).state, "failure");
});
test("an unrelated commenter cannot sign for the contributor", async () => {
  const f = fixture({ comments: [signature(bob)] });
  assert.equal((await f.run()).state, "failure");
  assert.equal(signaturesWritten(f).length, 0);
});

for (const eventName of ["issue_comment", "pull_request_target"]) {
  test(`${eventName} succeeds without a CLA for MIT-only changes`, async () => {
    const f = fixture({ files: [{ filename: "crates/tokscale-core/src/lib.rs" }], storageError: 404 });
    const event = eventName === "issue_comment" ? { ...f.event, action: "created", issue: { number: 42, pull_request: {} }, comment: { body: SIGN_TEXT } } : f.event;
    assert.deepEqual(await f.run({ eventName, event }), { state: "success", inScope: false });
    assert.deepEqual(statusStates(f), ["pending", "success"]);
    assert.ok(!f.calls.some((call) => call.path === "/graphql" || call.path.includes("/contents/")));
    assert.ok(!writes(f).some((call) => call.path.includes("/comments")));
  });
  test(`${eventName} finds frontend changes after the first 100 files`, async () => {
    const f = fixture({ files: [...Array.from({ length: 100 }, (_, i) => ({ filename: `docs/${i}.md` })), { filename: "packages/frontend/package.json" }] });
    const event = eventName === "issue_comment" ? { ...f.event, action: "created", issue: { number: 42, pull_request: {} }, comment: { body: "recheck" } } : f.event;
    assert.equal((await f.run({ eventName, event })).state, "failure");
    assert.equal(f.calls.filter((call) => call.path.endsWith("/files")).length, 2);
  });
}
test("renaming a file out of the frontend still requires the CLA", async () => {
  const f = fixture({ files: [{ filename: "docs/moved.md", previous_filename: "packages/frontend/README.md" }] });
  assert.equal((await f.run()).state, "failure");
});
test("deleted frontend files require the CLA", async () => {
  const f = fixture({ files: [{ filename: "packages/frontend/README.md", status: "removed" }] });
  assert.equal((await f.run()).state, "failure");
});
test("API's changed-files cap cannot silently classify a PR as out of scope", async () => {
  const f = fixture({ files: [{ filename: "docs/one.md" }], pr: { changed_files: 3001 } });
  await assert.rejects(f.run(), /every changed file/);
  assert.deepEqual(statusStates(f), ["pending", "error"]);
});

for (const merged of [false, true]) {
  test(`closed PR merged=${merged} is never signed, locked, or commented on`, async () => {
    const f = fixture({ pr: { state: "closed", merged }, comments: [signature()] });
    assert.equal((await f.run()).state, "skipped");
    assert.equal(writes(f).length, 0);
    const closedEvent = fixture({ event: { action: "closed" } });
    assert.equal((await closedEvent.run()).state, "skipped");
    assert.equal(closedEvent.calls.length, 0);
  });
}
for (const event of [
  { action: "created", issue: { number: 42 }, comment: { body: SIGN_TEXT } },
  { action: "edited", issue: { number: 42, pull_request: {} }, comment: { body: SIGN_TEXT } },
  { action: "created", issue: { number: 42, pull_request: {} }, comment: { body: `> ${SIGN_TEXT}` } },
]) {
  test(`irrelevant issue/comment event has no effects: ${JSON.stringify(event)}`, async () => {
    const f = fixture({ event });
    assert.equal((await f.run({ eventName: "issue_comment" })).state, "skipped");
    assert.equal(f.calls.length, 0);
  });
}

test("numeric 404 reports both missing branch and JSON setup without creating storage", async () => {
  const f = fixture({ storageError: 404 });
  await assert.rejects(f.run(), /Create cla-signatures AND seed signatures\/cla.json/);
  assert.equal(signaturesWritten(f).length, 0);
  assert.deepEqual(statusStates(f), ["pending", "error"]);
});
test("a seeded but empty storage accepts its first exact signature", async () => {
  const f = fixture({ comments: [signature()], document: { signedContributors: [] } });
  assert.equal((await f.run()).state, "success");
  assert.equal(signaturesWritten(f).length, 1);
});
for (const status of [401, 403, 429, 500]) {
  test(`HTTP ${status} is not mistaken for empty storage`, async () => {
    const f = fixture({ storageError: status });
    await assert.rejects(f.run(), (error) => error instanceof GitHubApiError && error.status === status);
    assert.equal(signaturesWritten(f).length, 0);
    assert.deepEqual(statusStates(f), ["pending", "error"]);
  });
}
test("malformed signature storage fails closed", async () => {
  const f = fixture({ document: { signedContributors: [{ id: "2", name: "alice" }] } });
  await assert.rejects(f.run(), /Invalid CLA storage/);
  assert.equal(signaturesWritten(f).length, 0);
  assert.throws(() => parseStorage({ type: "dir" }), /not a base64 GitHub file/);
});
test("storage conflicts cannot turn an unrecorded signature green", async () => {
  const f = fixture({ comments: [signature()], writeConflict: true });
  await assert.rejects(f.run(), (error) => error.status === 409);
  assert.deepEqual(statusStates(f), ["pending", "error"]);
});
test("a head change during collection fails closed", async () => {
  const f = fixture({ graphHead: "d".repeat(40) });
  await assert.rejects(f.run(), /changed while reading commits/);
  assert.deepEqual(statusStates(f), ["pending", "error"]);
});
test("a head change before persistence does not record a signature", async () => {
  const f = fixture({ comments: [signature()], changedPr: { head: { sha: "d".repeat(40) } } });
  await assert.rejects(f.run(), /changed during/);
  assert.equal(signaturesWritten(f).length, 0);
});
test("incomplete and repeating commit pagination fail closed", async () => {
  const commits = Array.from({ length: 101 }, (_, i) => commit([owner], i + 1));
  await assert.rejects(fixture({ commits, truncateCommits: true }).run(), /every pull request commit/);
  await assert.rejects(fixture({ commits, repeatCursor: true }).run(), /did not advance/);
});
test("the existing bot prompt on a late comments page is updated, not duplicated", async () => {
  const comments = [...Array.from({ length: 100 }, (_, i) => signature(alice, `discussion ${i}`, i + 1)),
    signature(bot, "<!-- tokscale-cla:v1 -->\nold prompt", 101)];
  const f = fixture({ comments });
  assert.equal((await f.run()).state, "failure");
  assert.equal(writes(f).filter((call) => call.path.endsWith("/comments")).length, 0);
  assert.ok(writes(f).some((call) => call.method === "PATCH" && call.path.endsWith("/comments/101")));
});
test("a human cannot impersonate the CLA bot prompt", async () => {
  const f = fixture({ comments: [signature(alice, "<!-- tokscale-cla:v1 -->\nspoof", 101)] });
  await f.run();
  assert.ok(!writes(f).some((call) => call.method === "PATCH"));
});
test("dry run reads metadata but never changes GitHub state", async () => {
  const f = fixture({ comments: [signature()] });
  assert.equal((await f.run({ dryRun: true })).state, "success");
  assert.equal(writes(f).length, 0);
});
test("unexpected repository/event inputs are rejected", async () => {
  const f = fixture();
  await assert.rejects(f.run({ repository: "other/repo" }), /Unexpected GitHub repository/);
  await assert.rejects(f.run({ event: { ...f.event, pull_request: { number: "42; echo unsafe" } } }), /Invalid pull request number/);
  assert.equal(f.calls.length, 0);
});
test("initializer only creates a new local seed and refuses to overwrite signatures", async () => {
  const dir = await mkdtemp(join(tmpdir(), "tokscale-cla-test-"));
  try {
    const path = join(dir, "signatures/cla.json");
    await initializeStorage(path);
    assert.deepEqual(JSON.parse(await readFile(path, "utf8")), { signedContributors: [] });
    await assert.rejects(initializeStorage(path), (error) => error.code === "EEXIST");
    assert.deepEqual(JSON.parse(await readFile(path, "utf8")), { signedContributors: [] });
  } finally { await rm(dir, { recursive: true, force: true }); }
});
test("transport rejects partial GraphQL errors and numeric REST API errors", async () => {
  const graph = createGitHubClient("token", async () => response({ data: { partial: true }, errors: [{ message: "rate limit" }] }));
  await assert.rejects(graph.graphql("query {}", {}), /GraphQL returned errors/);
  const rest = createGitHubClient("token", async () => response({}, 404));
  await assert.rejects(rest.request("GET", "/repos/junhoyeo/tokscale"), (error) => error.status === 404);
});
test("transport never sends the token to another host or list path", async () => {
  let requests = 0;
  const api = createGitHubClient("token", async () => { requests++; return response([], 200,
    { link: '<https://evil.invalid/comments?page=2>; rel="next"' }); });
  await assert.rejects(api.request("GET", "https://evil.invalid/"), /outside api.github.com/);
  assert.equal(requests, 0);
  await assert.rejects(api.list("/repos/junhoyeo/tokscale/issues/42/comments"), /outside this list/);
  assert.equal(requests, 1);
});
test("workflow only runs trusted default-branch code with minimal write scopes", async () => {
  const workflow = await readFile(new URL("../../workflows/cla.yml", import.meta.url), "utf8");
  assert.match(workflow, /ref: refs\/heads\/\$\{\{ github.event.repository.default_branch \}\}/);
  assert.match(workflow, /persist-credentials: false/);
  assert.match(workflow, /actions\/checkout@[a-f0-9]{40}/);
  assert.doesNotMatch(workflow, /actions: write|issues: write|pull_request\.head|pull_request\.merge_commit_sha/);
  assert.doesNotMatch(workflow, /types:.*closed|^\s+paths:/m);
  assert.deepEqual(workflow.match(/^\s+run:.+/gm)?.map((line) => line.trim()), [
    "run: node .github/scripts/cla/check.mjs --resolve-head", "run: node .github/scripts/cla/check.mjs",
  ]);
  assert.match(workflow, /group: cla-\$\{\{ github.repository \}\}-\$\{\{ needs.resolve.outputs.sha \}\}/);
  assert.match(workflow, /CLA_EXPECTED_HEAD: \$\{\{ needs.resolve.outputs.sha \}\}/);
});

test("simultaneous signatures merge after a compare-and-swap conflict", async () => {
  const f = fixture({ comments: [signature()], conflictOnce: true,
    concurrentDocument: { signedContributors: [{ id: bob.databaseId, name: bob.login }] } });
  assert.equal((await f.run()).state, "success");
  const puts = f.calls.filter((call) => call.method === "PUT");
  assert.equal(puts.length, 2);
  const saved = JSON.parse(Buffer.from(puts[1].body.content, "base64").toString("utf8"));
  assert.deepEqual(saved.signedContributors.map((user) => user.id), [bob.databaseId, alice.databaseId]);
});
test("persistent conflicts are bounded to three writes", async () => {
  const f = fixture({ comments: [signature()], writeConflict: true });
  await assert.rejects(f.run(), (error) => error.status === 409);
  assert.equal(f.calls.filter((call) => call.method === "PUT").length, 3);
});
for (const inScope of [true, false]) {
  test(`same-head PRs cannot overwrite the shared status with a false success (frontend=${inScope})`, async () => {
    const openPrs = [{ number: 42, head: { sha: SHA } }, ...Array.from({ length: 100 }, (_, i) =>
      ({ number: i + 100, head: { sha: i.toString(16).padStart(40, "0") } })), { number: 43, head: { sha: SHA } }];
    const f = fixture({ openPrs, files: [{ filename: inScope ? "packages/frontend/package.json" : "README.md" }], comments: [signature()] });
    await assert.rejects(f.run(), /Multiple open pull requests share/);
    assert.deepEqual(statusStates(f), ["pending", "error"]);
    assert.equal(signaturesWritten(f).length, 0);
    assert.equal(f.calls.filter((call) => call.path.endsWith("/pulls")).length, 2);
  });
}
test("retargeting a PR rechecks its frontend scope", async () => {
  const f = fixture({ event: { action: "edited" } });
  assert.equal((await f.run()).state, "failure");
});

test("read-only scheduling preflight only reads and validates the current PR head", async () => {
  const f = fixture();
  const target = await loadPullRequest({ api: f.api, eventName: "pull_request_target", event: f.event, repository: REPO });
  assert.equal(target.pr.head.sha, SHA);
  assert.equal(f.calls.length, 1);
  assert.equal(writes(f).length, 0);
});
test("the privileged run never writes for a different head than its serialized job", async () => {
  const f = fixture({ comments: [signature()] });
  await assert.rejects(f.run({ expectedHead: "e".repeat(40) }), /changed after scheduling/);
  assert.equal(writes(f).length, 0);
});

for (const frontend of [true, false]) {
  test(`a stale signature prompt is cleared after the contributors/scope change (frontend=${frontend})`, async () => {
    const f = fixture({ commits: [commit([owner])], files: [{ filename: frontend ? "packages/frontend/README.md" : "README.md" }],
      comments: [signature(bot, "<!-- tokscale-cla:v1 -->\nAwaiting @alice", 77)] });
    assert.equal((await f.run()).state, "success");
    const patch = writes(f).find((call) => call.method === "PATCH");
    assert.ok(patch);
    assert.doesNotMatch(patch.body.body, /Awaiting/);
    assert.equal(signaturesWritten(f).length, 0);
  });
}

test("real GitHub numeric-repository pagination Links retain scope and late signatures", async () => {
  const f = fixture({ canonicalLinks: true,
    files: [...Array.from({ length: 100 }, (_, i) => ({ filename: `docs/${i}.md` })), { filename: "packages/frontend/package.json" }],
    comments: [...Array.from({ length: 130 }, (_, i) => signature(alice, `discussion ${i}`, i + 1)), signature(alice, SIGN_TEXT, 131)],
  });
  assert.equal((await f.run()).state, "success");
  assert.equal(signaturesWritten(f)[0].comment_id, 131);
  assert.ok(f.calls.some((call) => call.path === "/repositories/101/pulls/42/files"));
  assert.ok(f.calls.some((call) => call.path === "/repositories/101/issues/42/comments"));
});
test("numeric pagination aliases cannot change repository identity", async () => {
  const api = createGitHubClient("token", async () => response([], 200,
    { link: '<https://api.github.com/repositories/999/issues/42/comments?per_page=100&page=2>; rel="next"' }));
  await assert.rejects(api.list("/repos/junhoyeo/tokscale/issues/42/comments", 101), /outside this list/);
});
test("pagination cannot drop a list filter", async () => {
  const api = createGitHubClient("token", async () => response([], 200,
    { link: '<https://api.github.com/repositories/101/pulls?per_page=100&page=2>; rel="next"' }));
  await assert.rejects(api.list("/repos/junhoyeo/tokscale/pulls?state=open", 101), /changed a list filter/);
});
test("repository IDs are checked against current GitHub PR metadata", async () => {
  const f = fixture();
  await assert.rejects(f.run({ event: { ...f.event, repository: { full_name: REPO, id: 999 } } }), /Unexpected GitHub pull request/);
  assert.equal(writes(f).length, 0);
});
