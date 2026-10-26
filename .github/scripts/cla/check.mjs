import { appendFile, readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { createGitHubClient } from "./github.mjs";

export const SIGN_TEXT = "I have read the CLA Document and I hereby sign the CLA";
export const STATUS_CONTEXT = "CLA / frontend";
export const STORAGE_BRANCH = "cla-signatures";
export const STORAGE_PATH = "signatures/cla.json";
const MARKER = "<!-- tokscale-cla:v1 -->";
const OWNER = "junhoyeo";
const USER_FIELDS = "login databaseId __typename";
export const COMMITS_QUERY = `query($owner:String!, $name:String!, $number:Int!, $cursor:String) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      headRefOid
      commits(first:100, after:$cursor) {
        totalCount
        nodes { commit { id oid authors(first:100) {
          nodes { user { ${USER_FIELDS} } }
          pageInfo { hasNextPage endCursor }
        } } }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}`;
const AUTHORS_QUERY = `query($id:ID!, $cursor:String!) {
  node(id:$id) { ... on Commit { authors(first:100, after:$cursor) {
    nodes { user { ${USER_FIELDS} } }
    pageInfo { hasNextPage endCursor }
  } } }
}`;

function validId(id) {
  return Number.isSafeInteger(id) && id > 0;
}
function validLogin(login) {
  return typeof login === "string" && /^[a-z\d](?:[a-z\d-]{0,38}|[a-z\d-]{0,33}\[bot\])$/i.test(login);
}
function nextCursor(connection, visited) {
  if (!connection?.pageInfo || !Array.isArray(connection.nodes)) {
    throw new Error("Incomplete GitHub connection response.");
  }
  if (!connection.pageInfo.hasNextPage) return null;
  const cursor = connection.pageInfo.endCursor;
  if (typeof cursor !== "string" || !cursor || visited.has(cursor)) {
    throw new Error("GitHub cursor pagination did not advance.");
  }
  visited.add(cursor);
  return cursor;
}
function eventPullNumber(eventName, event) {
  if (eventName === "pull_request_target" && ["opened", "reopened", "synchronize", "ready_for_review", "edited"].includes(event.action)) {
    return event.pull_request?.number;
  }
  if (eventName === "issue_comment" && event.action === "created" && event.issue?.pull_request &&
      [SIGN_TEXT, "recheck"].includes(event.comment?.body)) {
    return event.issue.number;
  }
  return null;
}

export async function collectContributors(api, repository, number, pr) {
  const [owner, name] = repository.split("/");
  const users = new Map();
  const unknown = new Set();
  function add(user, source) {
    if (!user || !validId(user.databaseId) || !validLogin(user.login)) {
      unknown.add(source);
    } else if (user.login.toLowerCase() !== OWNER && user.__typename !== "Bot" && !user.login.endsWith("[bot]")) {
      // GitActor.user is a GraphQL User even for bots. GitHub reserves the
      // bracketed [bot] login suffix; never infer a bot from a git author name.
      users.set(user.databaseId, { id: user.databaseId, name: user.login });
    }
  }
  add(pr.user && { databaseId: pr.user.id, login: pr.user.login, __typename: pr.user.type }, "pull request author");
  let cursor = null;
  let count = 0;
  const visited = new Set();
  do {
    const data = await api.graphql(COMMITS_QUERY, { owner, name, number, cursor });
    const pull = data.repository?.pullRequest;
    if (pull?.headRefOid !== pr.head.sha || pull?.commits?.totalCount !== pr.commits) {
      throw new Error("Pull request changed while reading commits; post recheck.");
    }
    const commits = pull.commits;
    cursor = nextCursor(commits, visited);
    for (const node of commits.nodes) {
      const commit = node.commit;
      if (!commit?.id || !/^[a-f\d]{40}$/.test(commit.oid)) throw new Error("Invalid GitHub commit response.");
      count++;
      let authors = commit.authors;
      const authorCursors = new Set();
      let authorCursor;
      do {
        authorCursor = nextCursor(authors, authorCursors);
        if (!authors.nodes.length) unknown.add(`commit ${commit.oid}`);
        for (const author of authors.nodes) add(author.user, `commit ${commit.oid}`);
        if (authorCursor) {
          const page = await api.graphql(AUTHORS_QUERY, { id: commit.id, cursor: authorCursor });
          authors = page.node?.authors;
        }
      } while (authorCursor);
    }
  } while (cursor);
  if (count !== pr.commits) throw new Error("GitHub did not return every pull request commit.");
  return { users: [...users.values()], unknown: [...unknown] };
}

function findPrompt(comments) {
  return comments.find((comment) => comment.user?.type === "Bot" &&
    comment.user.login === "github-actions[bot]" && comment.body?.startsWith(MARKER));
}

export function parseStorage(file) {
  if (file.type !== "file" || file.encoding !== "base64" || !/^[a-f\d]{40}$/.test(file.sha)) {
    throw new Error("CLA storage is not a base64 GitHub file.");
  }
  const document = JSON.parse(Buffer.from(file.content, "base64").toString("utf8"));
  if (!Array.isArray(document.signedContributors) || document.signedContributors.some((entry) =>
    !validId(entry.id) || !validLogin(entry.name))) {
    throw new Error("Invalid CLA storage schema; do not overwrite existing signatures.");
  }
  return { document, sha: file.sha };
}

export async function saveSignatures(api, storagePath, initial, newSigned, number) {
  let storage = initial;
  for (let attempt = 0; attempt < 3; attempt++) {
    const ids = new Set(storage.document.signedContributors.map((entry) => entry.id));
    const additions = newSigned.filter((entry) => !ids.has(entry.id));
    if (!additions.length) return storage;
    const document = { ...storage.document, signedContributors: [...storage.document.signedContributors, ...additions] };
    try {
      await api.request("PUT", storagePath, {
        branch: STORAGE_BRANCH, sha: storage.sha,
        message: `docs(cla): record signatures for #${number}`,
        content: Buffer.from(JSON.stringify(document, null, 2) + "\n").toString("base64"),
      });
      return { ...storage, document };
    } catch (error) {
      // A bounded compare-and-swap retry merges other PRs' new signatures.
      // Other API errors are not missing/empty storage and must fail closed.
      if (error.status !== 409 || attempt === 2) throw error;
      const { data } = await api.request("GET", `${storagePath}?ref=${STORAGE_BRANCH}`);
      storage = parseStorage(data);
    }
  }
}

export async function loadPullRequest({ api, eventName, event, repository }) {
  if (!/^[a-z\d-]+\/[a-z\d_.-]+$/i.test(repository) || event.repository?.full_name !== repository ||
      !validId(event.repository.id)) throw new Error("Unexpected GitHub repository input.");
  const number = eventPullNumber(eventName, event);
  if (number === null) return null;
  if (!validId(number)) throw new Error("Invalid pull request number.");
  const prefix = `/repos/${repository}`;
  const pullPath = `${prefix}/pulls/${number}`;
  const { data: pr } = await api.request("GET", pullPath);
  if (pr.base?.repo?.full_name !== repository || pr.base.repo.id !== event.repository.id ||
      !/^[a-f\d]{40}$/.test(pr.head?.sha)) {
    throw new Error("Unexpected GitHub pull request response.");
  }
  // Closed/merged PRs are never signed, commented on, or locked.
  if (pr.state !== "open") return null;
  return { pr, number, prefix, pullPath };
}

export async function runCla({ api, eventName, event, repository, dryRun = false, expectedHead }) {
  const target = await loadPullRequest({ api, eventName, event, repository });
  if (!target) return { state: "skipped" };
  const { pr, number, prefix, pullPath } = target;
  if (expectedHead !== undefined && expectedHead !== pr.head.sha) {
    throw new Error("Pull request head changed after scheduling; post recheck.");
  }
  async function status(state, description) {
    if (!dryRun) await api.request("POST", `${prefix}/statuses/${pr.head.sha}`, {
      state, context: STATUS_CONTEXT, description,
      target_url: `https://github.com/${repository}/blob/main/CLA.md`,
    });
  }
  async function assertUnchanged() {
    const { data: current } = await api.request("GET", pullPath);
    if (current.state !== "open" || current.head?.sha !== pr.head.sha || current.base?.sha !== pr.base.sha) {
      throw new Error("Pull request changed during the CLA check; post recheck.");
    }
    // Statuses belong to commits, not PRs. Another PR with the same head but
    // different base/scope must not inherit an out-of-scope successful status.
    // The commits/{sha}/pulls endpoint omits some open PRs for default-branch
    // commits; use the full open-PR list instead.
    const open = await api.list(`${prefix}/pulls?state=open`, event.repository.id);
    if (open.some((pull) => !validId(pull.number) || !/^[a-f\d]{40}$/.test(pull.head?.sha)) ||
        !open.some((pull) => pull.number === number && pull.head.sha === pr.head.sha)) {
      throw new Error("Incomplete open pull request metadata; post recheck.");
    }
    if (open.some((pull) => pull.number !== number && pull.head.sha === pr.head.sha)) {
      throw new Error("Multiple open pull requests share this head commit. Use distinct head commits, then post recheck; see docs/cla-setup.md.");
    }
  }
  await status("pending", "Checking frontend scope and contributor signatures.");
  try {
    const files = await api.list(`${pullPath}/files`, event.repository.id);
    if (!Number.isSafeInteger(pr.changed_files) || files.length !== pr.changed_files ||
        files.some((file) => typeof file.filename !== "string")) {
      // REST caps this endpoint at 3,000 files. Never treat a truncated result as out of scope.
      throw new Error("GitHub did not return every changed file; maintainer review required.");
    }
    const inScope = files.some((file) => [file.filename, file.previous_filename]
      .some((path) => typeof path === "string" && path.startsWith("packages/frontend/")));
    if (!inScope) {
      await assertUnchanged();
      const comments = await api.list(`${prefix}/issues/${number}/comments`, event.repository.id);
      const prompt = findPrompt(comments);
      const body = `${MARKER}\nThis pull request no longer changes the frontend. A CLA is not required.`;
      if (!dryRun && prompt && prompt.body !== body) {
        await api.request("PATCH", `${prefix}/issues/comments/${prompt.id}`, { body });
      }
      await assertUnchanged();
      await status("success", "No frontend changes; a CLA is not required.");
      return { state: "success", inScope: false };
    }
    const contributors = await collectContributors(api, repository, number, pr);
    let storage;
    const comments = await api.list(`${prefix}/issues/${number}/comments`, event.repository.id);
    const newSigned = [];
    const storagePath = `${prefix}/contents/${STORAGE_PATH}`;
    if (contributors.users.length) {
      try {
        const { data: file } = await api.request("GET", `${storagePath}?ref=${STORAGE_BRANCH}`);
        storage = parseStorage(file);
      } catch (error) {
        if (error.status === 404) {
          throw new Error("CLA storage is not initialized. Create cla-signatures AND seed signatures/cla.json; see docs/cla-setup.md.");
        }
        throw error;
      }
      const signed = new Set(storage.document.signedContributors.map((entry) => entry.id));
      const unsigned = new Map(contributors.users.filter((user) => !signed.has(user.id)).map((user) => [user.id, user]));
      for (const comment of comments) {
        const user = unsigned.get(comment.user?.id);
        // Strict equality: no substring matching, markdown stripping, case folding, or trimming.
        if (user && comment.user.type === "User" && comment.body === SIGN_TEXT && validId(comment.id) &&
            typeof comment.created_at === "string" && Number.isFinite(Date.parse(comment.created_at))) {
          newSigned.push({ ...user, comment_id: comment.id, created_at: comment.created_at,
            repoId: event.repository.id, pullRequestNo: number, body: comment.body,
            commentUrl: `https://github.com/${repository}/pull/${number}#issuecomment-${comment.id}` });
          unsigned.delete(user.id);
        }
      }
      await assertUnchanged();
      if (newSigned.length && !dryRun) {
        storage = await saveSignatures(api, storagePath, storage, newSigned, number);
      }
    }
    const signedIds = new Set([...(storage?.document.signedContributors ?? []), ...newSigned].map((user) => user.id));
    const unsigned = contributors.users.filter((user) => !signedIds.has(user.id));
    const passed = !unsigned.length && !contributors.unknown.length;
    const prompt = findPrompt(comments);
    const body = passed
      ? `${MARKER}\nAll identified contributors have signed the frontend CLA or are exempt.`
      : `${MARKER}\nThis pull request changes \`packages/frontend/\`. Please read the [CLA](https://github.com/${repository}/blob/main/CLA.md) (currently a draft for legal review).\n\n` +
        (unsigned.length ? `Awaiting a signature from: ${unsigned.map((user) => `@${user.name}`).join(", ")}.\n\nPost this exact text as the entire comment (no quotes or extra text):\n\n${SIGN_TEXT}\n\n` : "") +
        (contributors.unknown.length ? "Some commit authors are not linked to a GitHub account. Link their commit email to the correct GitHub account, then post `recheck`. A signed committer is not a substitute for an unidentified author.\n\n" : "") +
        "Signatures are public and apply to later frontend pull requests. Post `recheck` to run this check again.";
    if (!dryRun && (!passed || prompt) && prompt?.body !== body) {
      if (prompt) await api.request("PATCH", `${prefix}/issues/comments/${prompt.id}`, { body });
      else await api.request("POST", `${prefix}/issues/${number}/comments`, { body });
    }
    await assertUnchanged();
    await status(passed ? "success" : "failure", passed ? "All frontend contributors have signed or are exempt." :
      "Frontend contributors must sign the CLA; see the pull request comment.");
    return { state: passed ? "success" : "failure", inScope: true, unsigned, unknown: contributors.unknown, newSigned };
  } catch (error) {
    await status("error", "CLA check could not complete; see workflow logs and docs/cla-setup.md.");
    throw error;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const args = process.argv.slice(2);
    if (args.length > 1 || args.some((arg) => !["--dry-run", "--resolve-head"].includes(arg))) {
      throw new Error("Only --dry-run or --resolve-head is supported.");
    }
    const event = JSON.parse(await readFile(process.env.GITHUB_EVENT_PATH, "utf8"));
    const inputs = { api: createGitHubClient(process.env.GITHUB_TOKEN),
      eventName: process.env.GITHUB_EVENT_NAME, event, repository: process.env.GITHUB_REPOSITORY };
    if (args.includes("--resolve-head")) {
      // The read-only preflight exposes only a validated hex SHA, not PR text.
      const target = await loadPullRequest(inputs);
      await appendFile(process.env.GITHUB_OUTPUT, `sha=${target?.pr.head.sha ?? ""}\n`);
    } else {
      const result = await runCla({ ...inputs, dryRun: args.includes("--dry-run"), expectedHead: process.env.CLA_EXPECTED_HEAD });
      console.log(JSON.stringify(result, null, 2));
      if (result.state === "failure") process.exitCode = 1;
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
