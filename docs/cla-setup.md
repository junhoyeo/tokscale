# Frontend CLA check: maintainer setup

The CLA and frontend licensing documents remain drafts for legal review. This automation checks recorded consent; it does not resolve legal scope or grant rights by itself. The owner must approve the documents and enable the check. A folder boundary or separate deployment does not by itself determine whether services are separate works under the AGPL. Do not promise guaranteed AGPL isolation from this repository layout.

## Initialize storage before enabling the workflow

Both the `cla-signatures` branch **and** `signatures/cla.json` must exist. An empty branch is not sufficient. The workflow deliberately does not create a branch or replace missing/corrupt storage. A numeric GitHub HTTP 404 fails with setup instructions; 401, 403, rate limits, and other errors are not treated as an empty signature list.

These are explicit owner-operated steps, not actions performed by the workflow or initializer:

1. Keep the workflow disabled in GitHub Actions until legal review and storage setup are complete. No product environment toggle is used.
2. Inspect the effective Git identity (`git config user.name` and `git config user.email`) and the `origin` remote. Commits must use `Junho Yeo <i@junho.io>`. Check whether the remote branch already exists. If it does, inspect its current JSON and preserve every existing signature; do not run a bootstrap that replaces it.
3. If the branch does not exist, fetch the trusted default branch and create a **new, dedicated worktree**, for example `git worktree add -b cla-signatures ../tokscale-cla-signatures origin/main`. Do not switch or clear a shared checkout. The branch can start from `main`; an orphan branch is not required.
4. From the reviewed source checkout, run `node .github/scripts/cla/initialize.mjs ../tokscale-cla-signatures/signatures/cla.json`. The initializer only creates a local JSON file. It refuses to overwrite an existing file, and never creates a branch, contacts GitHub, commits, or pushes. The seed is:

   ```json
   { "signedContributors": [] }
   ```

5. In the new worktree, inspect the diff, stage **only** `signatures/cla.json`, and commit it as `docs(cla): initialize signature storage`. The owner can then explicitly push **that branch only** with `git push origin cla-signatures`. No PAT is needed for the same-repository workflow.
6. Read the remote branch and the Contents API response for `signatures/cla.json?ref=cla-signatures`. Confirm the branch, file, and JSON shape actually exist. Protect the signature branch from untrusted/manual replacement while allowing this repository's `GITHUB_TOKEN` to update that file. Repository rules can deny these updates; confirm the allowed write path rather than broadly disabling protections.
7. Make the reviewed workflow and scripts available on the trusted default branch. Enable the workflow and run the acceptance checks below before making a status required. These steps are intentionally left to the owner; a successful local test is not proof of live workflow setup.

If a seed or signature file already exists, **stop and inspect it** instead of deleting or regenerating it. Existing `signedContributors` entries use numeric GitHub account IDs and login names. New records also retain the exact consent body, comment ID/URL, creation timestamp, repository ID, and PR number. Treat this branch as public signature evidence and keep its history.

## Workflow behavior and trust boundary

- The privileged job checks out only `refs/heads/<repository default branch>` from the base repository, using an immutable `actions/checkout` ref with credentials not persisted. It never checks out a PR head or merge ref, installs PR dependencies, imports PR modules, or expands comment text into a shell command. Review changes to the trusted script before merging them to the default branch.
- Both `pull_request_target` and `issue_comment` get the current PR and its changed files from GitHub. Changes, deletions, and moves into or out of `packages/frontend/` require the CLA. MIT-only PRs receive an explicit successful no-op. No workflow-level `paths` filter can leave a required status permanently pending.
- Every PR commit page, each commit's author/coauthor pages, and every PR comment/file page are read. REST's changed-file limit (3,000) or an incomplete result fails closed. Unknown GitHub authors must link their commit email to the correct account; a signed committer does not substitute for the author.
- The PR author and linked commit authors/coauthors must sign once. `junhoyeo` and GitHub bot accounts are exempt. GitHub's GraphQL `GitActor.user` is a `User` even for bots, so the check also recognizes GitHub's reserved `[bot]` login suffix, not arbitrary git author names. Signatures are matched by numeric account ID, not a reusable login.
- Only an entire comment equal to `I have read the CLA Document and I hereby sign the CLA` is consent. Quotes, blockquotes, refusals, added lines, different casing, and leading/trailing whitespace do not match. A `recheck` comment requests reevaluation; it is not consent. Scanning on a later push uses the same exact rule.
- Both events ignore closed PRs. Closing an unmerged or merged PR never locks discussion. Reopening, retargeting, new commits, and ready-for-review events recheck the current state. Draft status does not bypass consent.
- The token has only `contents: write` (signature storage), `pull-requests: write` (metadata and PR discussion comments), and `statuses: write`. No `actions: write`, PAT, PR-branch write, empty commit, workflow rerun, or lock operation is needed. GitHub cannot narrow a `contents` permission to one path; trusted default-branch code and repository governance are part of this boundary.
- A read-only metadata job resolves a validated current head SHA. The privileged job serializes by that SHA, then checks it still matches before writing. This prevents unrelated PR traffic from replacing another head's pending job, and same-head checks from posting stale successes after one another. Different heads can sign concurrently. A Contents API SHA compare-and-swap retries HTTP 409 conflicts at most three times, refetching and merging signatures. Persistent conflicts and all other write errors fail rather than reporting an unrecorded signature as signed.

## Acceptance and required checks

GitHub commit statuses are shared by every PR with the same head SHA, even when their base branches or changed-file scopes differ. The check therefore fails closed when multiple open PRs share the exact head commit. Use distinct head commits (or close the duplicate PR), then post `recheck`. It paginates the repository's open PR list and checks again before publishing a result; it does not rely on the commit-to-PR endpoint's limited association rules. Same-SHA jobs are serialized, but GitHub can display an earlier commit status before a new PR event starts. Commit statuses are not atomic, PR-isolated legal gates; maintainers must not bypass the duplicate-head restriction based on an inherited earlier green status.

The commit status context is **`CLA / frontend`**, not the workflow job `CLA metadata check`. A signing comment updates the status on the PR's actual current head commit; it does not rerun an earlier workflow. If the owner chooses to require the CLA, configure this status only after verifying both events with the repository's actual token permissions.

A push to a PR's base branch does not itself trigger these events. Its merge base and changed-file scope can change without a new head SHA, so an earlier success can become stale. Before merging after a base update, a maintainer must post `recheck` and wait for a fresh result. Requiring branches to be up to date helps with many base changes but is not sufficient if the new base is already an ancestor of the head. Until base-update rechecks are automated, this status alone is not complete unattended merge enforcement. No branch-protection or merge rules are changed by this setup.

Before enabling it as required, verify in a disposable PR or controlled test repository:

1. An unsigned frontend contributor fails, and an exact signature records once and passes. A quote/refusal containing the sentence stays unsigned after a push and after `recheck`.
2. Owner/bot-only PRs pass. A previously signed account passes. An unsigned author/coauthor beyond commit 100 is still required. A valid signature beyond comments 30 and 100 is found.
3. CLI-only PRs pass without requesting or recording a signature, including when someone posts `recheck` or the signing sentence.
4. A missing branch, a branch without the file, corrupt JSON, denied writes, and conflicting writes fail with an actionable log. The seeded empty file handles its first signature without auto-creation.
5. Closed/unmerged PR discussions remain unlocked. A stale event cannot pass a newer head or a changed base.

Local regression command (Node built-ins only; no install required):

```sh
node --test .github/scripts/cla/*.test.mjs
```

`.github/workflows/cla_tests.yml` runs this suite on an unprivileged `pull_request` job with a read-only token. That job may test proposed PR code; the privileged CLA job never does.

For a read-only diagnostic, the check accepts `--dry-run` with the normal GitHub event JSON/path, repository, event name, and token inputs. This mode reads PR metadata and storage but publishes no statuses/comments/signatures. Do not supply an untrusted API endpoint or run PR code with a privileged token. Each HTTP request has a 20-second abort bound; the workflow has a ten-minute limit. There is no claim that mocked or dry-run tests verify live write permissions or legal validity.
