// No third-party runtime dependencies. All requests stay on api.github.com.
export class GitHubApiError extends Error {
  constructor(status, method, path) {
    super(`GitHub API ${method} ${path} failed (HTTP ${status}).`);
    this.status = status;
  }
}

export function createGitHubClient(token, fetchImpl = fetch) {
  if (!token) throw new Error("GITHUB_TOKEN is required.");
  const origin = "https://api.github.com";
  async function request(method, path, body) {
    const url = new URL(path, origin);
    if (url.origin !== origin || !path.startsWith("/")) {
      throw new Error("Refusing a GitHub request outside api.github.com.");
    }
    const response = await fetchImpl(url, {
      method,
      headers: {
        authorization: `Bearer ${token}`,
        accept: "application/vnd.github+json",
        "content-type": "application/json",
        "x-github-api-version": "2022-11-28",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(20_000),
      // Never forward the token to a redirect target.
      redirect: "error",
    });
    if (!response.ok) throw new GitHubApiError(response.status, method, url.pathname);
    return { data: await response.json(), headers: response.headers };
  }
  return {
    request,
    async graphql(query, variables) {
      const { data } = await request("POST", "/graphql", { query, variables });
      if (data.errors?.length || !data.data) throw new Error("GitHub GraphQL returned errors.");
      return data.data;
    },
    async list(path, repositoryId) {
      const items = [];
      const first = new URL(path, origin);
      if (first.origin !== origin || !path.startsWith("/")) throw new Error("Invalid GitHub list path.");
      first.searchParams.set("per_page", "100");
      // GitHub emits canonical numeric-repository URLs in Link headers, even
      // when the request used /repos/{owner}/{repo}. Only allow this exact ID
      // (validated against PR metadata), never an arbitrary repository alias.
      const resource = first.pathname.match(/^\/repos\/[^/]+\/[^/]+(\/.*)$/)?.[1];
      const canonical = resource && Number.isSafeInteger(repositoryId) && repositoryId > 0
        ? `/repositories/${repositoryId}${resource}` : null;
      let next = first.pathname + first.search;
      const visited = new Set();
      while (next) {
        if (visited.has(next)) throw new Error("GitHub pagination did not advance.");
        visited.add(next);
        const { data, headers } = await request("GET", next);
        if (!Array.isArray(data)) throw new Error("Expected a GitHub list response.");
        items.push(...data);
        const link = headers.get("link")?.split(",").find((part) => /;\s*rel="next"/.test(part));
        if (!link) break;
        const match = link.match(/<([^>]+)>/);
        if (!match) throw new Error("Invalid GitHub pagination link.");
        const url = new URL(match[1]);
        if (url.origin !== origin || (url.pathname !== first.pathname && url.pathname !== canonical)) {
          throw new Error("Refusing a GitHub pagination link outside this list.");
        }
        for (const [key, value] of first.searchParams) {
          if (key !== "page" && url.searchParams.get(key) !== value) {
            throw new Error("GitHub pagination changed a list filter.");
          }
        }
        next = url.pathname + url.search;
      }
      return items;
    },
  };
}
