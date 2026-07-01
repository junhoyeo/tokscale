# Sakana (Fugu) subscription usage

Tokscale can surface your Sakana (Fugu) subscription quota windows in the
`tokscale usage` command and the TUI Usage tab.

> **Heads up — this is a best-effort scraper, not an API.**
> Sakana exposes **no public usage/quota API**. After investigation, the only
> source of subscription-usage data is the authenticated billing console at
> `https://console.sakana.ai/billing`. That page is a Next.js app, but the
> rendered usage values are present in the served HTML of a plain authenticated
> `GET`, so tokscale fetches that HTML with your session cookie and parses it.
> Because it is layout-coupled, it can degrade or break if Sakana restructures
> the billing page.

## What it shows

- **Quota windows, not dollars.** Sakana subscription billing is a flat monthly
  fee (e.g. `$20/mo`), so there is no per-request spend to report. Tokscale shows
  the rolling **5-hour** and **Weekly** quota windows (percent used + reset
  time).
- The plan tier (`Standard` / `Pro` / `Max`), the monthly price, and the next
  renewal date are surfaced as plan metadata.

## Obtaining the session cookie

The console authenticates with NextAuth/Auth.js session cookies. You need to
copy the session-token cookie out of your browser:

1. Log in to `https://console.sakana.ai` in your browser.
2. Open DevTools → **Application** tab → **Cookies** → `https://console.sakana.ai`.
3. Find the session-token cookie(s). NextAuth splits large session tokens across
   numbered chunks, so you will usually see two:
   - `__Secure-authjs.session-token.0`
   - `__Secure-authjs.session-token.1`
4. Combine them into a single `Cookie`-header string, in order, separated by
   `; `:

   ```
   __Secure-authjs.session-token.0=<value0>; __Secure-authjs.session-token.1=<value1>
   ```

   (If your account only has a single un-chunked
   `__Secure-authjs.session-token` cookie, just use that one.)

## Configuring tokscale

Provide the cookie string via **either** of these (env var takes precedence):

- Environment variable:

  ```bash
  export SAKANA_SESSION_COOKIE='__Secure-authjs.session-token.0=...; __Secure-authjs.session-token.1=...'
  ```

- Or a file (raw cookie string, trimmed):

  ```bash
  mkdir -p ~/.config/tokscale
  # Create the file mode 600 from the start (umask 077) so the cookie is never
  # briefly world-readable in the window between writing and chmod.
  ( umask 077 && printf '%s' '__Secure-authjs.session-token.0=...; __Secure-authjs.session-token.1=...' \
      > ~/.config/tokscale/sakana-session )
  # Alternative: install -m 600 /dev/null ~/.config/tokscale/sakana-session first.
  ```

  The file lives in tokscale's config dir, which honors `TOKSCALE_CONFIG_DIR`
  (and `$XDG_CONFIG_HOME/tokscale` on Linux); `~/.config/tokscale` is just the
  common default.

Then:

```bash
tokscale usage
```

Sakana appears automatically when a session cookie is available.

## The cookie expires

Session cookies are short-lived. When the cookie expires (or is invalidated by
logging out), the billing endpoint returns a login page or a `401`/`403`.
Tokscale detects this and reports a clear error asking you to refresh
`SAKANA_SESSION_COOKIE` — it does **not** emit a bogus parse. When that happens,
repeat the steps above to copy a fresh cookie.

## Treat the cookie like a password

The session cookie grants access to your Sakana account. Store it like a secret:
keep `~/.config/tokscale/sakana-session` at mode `600`, and don't commit it or
paste it into shared shell history.
