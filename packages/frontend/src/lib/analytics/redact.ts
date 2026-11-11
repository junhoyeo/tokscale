const ALLOWED_QUERY_PARAMETERS = new Set([
  "utm_source",
  "utm_medium",
  "utm_campaign",
  "utm_term",
  "utm_content",
  "view",
  "period",
]);

function redactGroupPath(pathname: string): string {
  if (/^\/groups\/join\/[^/]+\/?$/.test(pathname)) {
    return pathname.replace(/^(\/groups\/join\/)[^/]+/, "$1[token]");
  }

  if (/^\/groups\/(?!new\/?$)[^/]+\/?$/.test(pathname)) {
    return pathname.replace(/^(\/groups\/)[^/]+/, "$1[slug]");
  }

  return pathname;
}

export function redactAnalyticsUrl(url: string): string {
  try {
    const parsed = new URL(url);
    parsed.pathname = redactGroupPath(parsed.pathname);
    parsed.hash = "";

    for (const key of [...parsed.searchParams.keys()]) {
      if (!ALLOWED_QUERY_PARAMETERS.has(key)) {
        parsed.searchParams.delete(key);
      }
    }

    return parsed.toString();
  } catch {
    return url.split(/[?#]/, 1)[0];
  }
}
