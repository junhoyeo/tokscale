# Design

## Source of truth

- Status: Active
- Last refreshed: 2026-07-12
- Primary product surfaces: Public user profiles at `/u/[username]`, the profile embed dialog, public README widgets at `/api/embed/[username]/svg`, the global leaderboard at `/leaderboard`, the group directory at `/leaderboard?view=groups`, group detail at `/groups/[slug]`, and the group create/join flows.
- Evidence reviewed: `packages/frontend/src/app/u/[username]/ProfilePageClient.tsx`, `packages/frontend/src/components/profile/`, `packages/frontend/src/lib/embed/`, `packages/frontend/src/app/api/embed/[username]/svg/route.ts`, `packages/frontend/src/app/(main)/leaderboard/`, `packages/frontend/src/app/(main)/groups/`, `packages/frontend/src/lib/leaderboard/`, `packages/frontend/src/lib/groups/`, `packages/frontend/src/components/layout/Navigation.tsx`, `packages/frontend/src/app/globals.css`, actual local API data for the leaderboard and public groups, and the compact content/usage references at `https://cho.sh/ko` and `https://cho.sh/ko/mini/usage`.
- Visual reference captures: `.omx/artifacts/visual-ralph/compact-profile/`, `.omx/artifacts/embed-redesign/`, and the desktop/mobile leaderboard and group baselines under `.omx/artifacts/groups-leaderboard/baseline/`.

## Brand

- Personality: Precise, technical, calm, and quietly competitive.
- Trust signals: Exact usage values, transparent time ranges, accessible data labels, visible freshness, and familiar GitHub identity.
- Avoid: Cosmic decoration inside application screens, oversized pill controls, repeated metrics, gratuitous 3D treatment, excessive gradients, dense card stacks, and copying another product's branding.

## Product goals

- Goals: Make profiles understandable in one viewport; make global and group rankings immediately scannable; preserve tokens, cost, time, rank, and role at every viewport; make group discovery and membership workflows feel like one service; make every embed template communicate one distinct usage story; and work cleanly from 320px through desktop.
- Non-goals: No database schema, authentication, settings, global landing/footer asset, or `/local` graph redesign in this pass. The leaderboard response may be narrowed to facts its ranking surfaces consume. No new dependency and no pixel-for-pixel clone of the reference sites.
- Success signals: A visitor reaches leaderboard data without crossing a promotional hero; top ranks and active scope scan in seconds; public groups are compact enough to compare; group detail never collides with navigation; mobile ranking rows retain their decisive metrics without horizontal scrolling; and profile, leaderboard, groups, and embeds share one visual grammar.

## Personas and jobs

- Primary personas: A developer reviewing their own activity; a visitor comparing public users; a team owner managing a scoped ranking; a member checking standing; and a maintainer debugging submitted usage.
- User jobs: Identify a person or group, understand ranking scope, compare decisive usage metrics, find a member, inspect personal standing, create or join a group, configure a truthful embed, and share the result.
- Key contexts of use: Wide desktop comparison, narrow mobile ranking checks, keyboard-only navigation, authenticated and anonymous group discovery, GitHub README rendering, and datasets with long names, sparse activity, many pages, or missing optional metrics.

## Information architecture

- Primary navigation: Existing Tokscale application navigation remains unchanged. `/leaderboard` uses link-based Users/Groups navigation so URLs, history, and server rendering stay authoritative.
- Core routes/screens: `/u/[username]` is the canonical public profile; `/leaderboard` is the compact global ranking; `/leaderboard?view=groups` is the directory and membership entry point; `/groups/[slug]` is the scoped ranking and invite-management view; `/groups/new` and `/groups/join/[token]` are focused single-task forms.
- Content hierarchy: Profiles retain identity → metrics → analysis. Global leaderboard uses title/scope → aggregate facts → range/search/sort → ranking → join instructions. Group directory uses purpose/action → owned/public filter → compact group list. Group detail uses identity/membership → scoped facts → period/search/sort → ranking → invite management when authorized.

## Design principles

- Data before decoration: Labels, values, trend, and range context carry the hierarchy.
- One fact, one home: Do not repeat tokens, cost, active time, or sessions across multiple cards.
- Lightest useful surface: Use whitespace and dividers first; reserve bordered panels for the identity overview, charts, Usage details, Token mix, and independently grouped datasets.
- Compact, not cramped: Desktop controls use 28–36px heights; mobile keeps 44–48px coarse-pointer targets without inflating visual chrome.
- Ranking before promotion: Application rankings begin with their title and data; the black-hole marketing hero does not appear on leaderboard or group routes.
- Preserve comparison context: Responsive rankings recompose into compact rows rather than hiding cost, tokens, rank, role, or useful all-time facts.
- Honest group identity: Missing group artwork uses a deterministic text monogram on a restrained surface, never an unrelated decorative gradient.
- Reference, not replica: Adopt the reference's narrow content measure, restrained borders, chart-first composition, and low-noise controls while retaining Tokscale typography, data, and blue accent.
- One widget, one job: Template differences come from information hierarchy and reading density, never costume, decorative metaphor, or renamed identical layouts.
- Tradeoffs: The public profile keeps its purpose-built responsive usage trend and adds an optional inline isometric contribution view using the same scoped calendar as 2D. It does not reuse the heavier `/local` graph container or decorative 3D embed card. Raw totals remain authoritative, the usage trend still defaults to a trailing average, and 2D remains the default contribution view.

## Visual language

- Color: Dark zinc-neutral canvas; raised surfaces only slightly lighter; translucent white borders; white/default/muted text with WCAG AA contrast; Tokscale blue for the single primary action and selected data emphasis; provider colors only in chart/legend context.
- Typography: Existing Figtree UI font and JetBrains Mono for code only. Page title 20–24px medium/semibold, section title 16–18px medium, body 14–16px, metadata 12–13px where it is supplementary rather than body copy. Numeric values use tabular figures.
- Spacing/layout rhythm: 4px base; common gaps 8/12/16/20/24px; application canvases max out at 1500px with responsive 16–32px gutters. Keep headline metrics left-packed in compact tracks instead of stretching sparse facts across the canvas. Ranking rows target 56–64px desktop height and an information-complete 76–92px mobile composition.
- Shape/radius/elevation: 8px controls, 12px panels, full radius only for badges/avatars where semantically appropriate. Dark application surfaces use borders, not shadows.
- Motion: Immediate color/background state changes; 120–160ms transform only for pressed controls; honor `prefers-reduced-motion`.
- Imagery/iconography: GitHub avatar with a subtle dark-surface outline at 72px mobile and 80px desktop. Give rank one compact accent-backed emphasis beside identity metadata. Reuse existing 16px application icons and source assets; avoid decorative icon containers.

## Components

- Existing components to reuse: `Navigation`, profile components, formatters in `lib/utils`, shared graph palettes/settings, `TabBar`, existing 16px icons, and established server-fetching patterns.
- New/changed components: A shared compact application shell and `ServiceFooter` for profile/ranking routes; compact `LeaderboardViewSelector`, aggregate fact strip, responsive global ranking rows, `GroupDirectory`, deterministic group mark, scoped group overview, responsive group ranking rows, and focused create/join surfaces. Existing profile analytics and embed components remain unchanged in this follow-up.
- Variants and states: Primary/secondary/ghost actions; active/inactive navigation and ranges; current-user and top-three rows; public/private/member/owner/admin group states; loading, empty, search-empty, error, pagination, copied-invite, and submitting states; desktop table and mobile ranking-list compositions.
- Chart contract: Render one stable stacked area per provider/model pair. Order provider groups and their models by raw scoped usage ascending so dominant bands remain on top; sort only the active tooltip rows descending. Use provider-level legend colors, deterministic model shades, 40% fills, 1px monotone boundaries, and no chart animation.
- Contribution contract: Render the complete requested UTC date range, including zero-valued outer days; derive 2D intensity and 3D height from the same token-scoped calendar; expose compact view and palette selectors; show a viewport-clamped daily tooltip on hover/focus; and let click, tap, Enter, or Space update one persistent token, cost, client, and model breakdown. Default that breakdown to the visible range end, preserve it across view and palette changes, retain roving keyboard navigation in both views, and expose a concise screen-reader summary.
- Embed contract: Keep the live preview visually primary, place dense settings in a viewport-contained scroll region, expose only options the selected renderer consumes, and trap/restore focus while the dialog is open. The eight 2D templates share one solid surface, identity header, divider, footer, type scale, and restrained semantic colors while using distinct data hierarchies; decorative gradients, glows, patterns, fake chrome, and metaphor-heavy ornament are excluded. The 3D contribution view remains a supported first-class renderer with its own compatible controls. Desktop uses preview/settings panes; mobile uses one body scroll.
- Embed hierarchy: `Overview` balances identity and the three canonical facts; `Token focus` gives tokens dominant scale; `Readout` is a genuinely terse monospace key/value view; `Contributions` makes the calendar the hero; `Rank focus` centers standing and percentile context; `Activity summary` compares measurable one-year activity signals; `Detailed stats` is the densest two-column fact sheet; `Compact list` is the narrowest scan-first ledger. Do not add template-name overlines, invented system labels, or explanatory slogans inside the SVG.
- Token/component ownership: Additive service tokens live in `src/app/globals.css`; profile composition and variants live in `src/components/profile/`. Shared `/local` graph components are out of scope.

## Accessibility

- Target standard: WCAG 2.2 AA.
- Keyboard/focus behavior: Visible focus rings; link semantics for ranking destinations; native buttons, inputs, selects, and checkboxes; complete keyboard access to view, period, sort, pagination, copy, and membership actions; existing chart and embed focus contracts remain intact.
- Contrast/readability: Muted text must remain at least 4.5:1 for normal text; provider color is never the only data label; body text is at least 16px on mobile.
- Screen-reader semantics: Structured headings, `dl` for facts, real table semantics on wide ranking views, equivalent labeled lists on narrow views, `aria-current` on navigation, labeled search and date controls, status/alert regions for async results, and the existing chart/embed semantics.
- Reduced motion and sensory considerations: Disable nonessential transform/animation under reduced motion; preserve labels and values independently of hue.

## Responsive behavior

- Supported breakpoints/devices: 320px mobile through wide desktop; primary checks at 390, 768, and 1024+ CSS pixels. The usage chart targets 224px height on mobile and 256px on desktop.
- Layout adaptations: Profile behavior remains as defined. Leaderboard and group pages use a 1500px application shell with 16–32px gutters and no promotional hero. Aggregate facts use compact divider-separated tracks on desktop and a two-column grid on mobile. Global and group rankings render semantic tables where space permits and information-complete linked rows on mobile; no page-level or nested horizontal scroller is required. Group directory cards become compact list-like tiles with bounded descriptions instead of fixed empty height. Identity/actions and control bars wrap into a single column without overlapping the fixed navigation. Create and join forms remain bounded while using full mobile width.
- Touch/hover differences: Coarse pointers receive at least 44px effective targets; chart selection works by tap and keyboard, with a compact detail panel below the chart. Fine pointers receive a clamped, internally scrollable floating tooltip; contribution cells expose the same value on hover and focus.

## Interaction states

- Loading: Preserve server rendering and add a profile-shaped route skeleton only if loading behavior is introduced.
- Empty: Keep identity and metrics visible, then explain that usage data has not been submitted yet and point profile owners to the submit command when appropriate.
- Error: Existing route-level not-found behavior remains; interactive copy/share failures use the current toast channel.
- Success: Share confirms copy; embed actions retain their current confirmation behavior.
- Disabled: Native controls expose disabled semantics and reduced contrast without becoming unreadable.
- Offline/slow network, if applicable: Server-rendered profile content remains usable; navigation session enrichment may arrive later without moving the main layout.

## Content voice

- Tone: Concise, factual, developer-oriented.
- Terminology: Use “tokens”, “cost”, “active days”, “submissions”, “providers”, “models”, and “devices” consistently.
- Microcopy rules: Sentence case for controls/table headings, punctuation on full explanatory sentences, no emoji, and no ambiguous chart labels such as “all-time history” when only the latest year of daily rows is present.

## Implementation constraints

- Framework/styling system: Next.js 16, React 19, and styled-components. No Sass/Tailwind/chart package is introduced.
- Design-token constraints: Add tokens without changing existing global aliases used by landing, leaderboard, settings, groups, or `/local`.
- Performance constraints: Keep chart and contribution geometry derivation memoized; allow normal profiles to retain their model bands, apply a high pathological series cap with an explicit remainder, render only one contribution view at a time, and preserve server data fetching and ISR. Ranking queries and API payloads include only displayed identity, rank, token, cost, time, role, scope, and pagination facts; submission counts and freshness metadata stay on profile-specific surfaces.
- Analytical constraints: Missing calendar dates are zero-valued. Lifetime defaults to a trailing 30-day average and finite ranges to a trailing 7-day average, with daily values available as an explicit display mode. Moving averages never alter raw range totals or stable series ranking.
- Compatibility constraints: Leave auth, database schema, profile APIs, and canonical profile redirects unchanged. The public leaderboard APIs intentionally omit unused submission-count and freshness fields. Do not mutate the shared `GraphContainer` behavior. Preserve all public embed template IDs and query parameters, XML escaping, CSP-compatible standalone SVG output, intrinsic template widths, and the classic fallback for invalid or omitted templates.
- Test/screenshot expectations: Preserve existing profile/embed coverage; add focused tests for view-link filter preservation and responsive ranking/group presentation helpers; run frontend tests, lint, typecheck, and build; capture `/leaderboard`, `/leaderboard?view=groups`, and a populated public `/groups/[slug]` with actual API data at 1440×1100 and 390×844; exercise search, period, sort, view, and primary group navigation; persist the visual verdict under `.omx/state/groups-leaderboard/ralph-progress.json` with a pass target of 90.

## Open questions

- [ ] Decide in a future pass whether the compact service language should extend to settings, navigation, and the decorative global footer; owner: product/design; impact: site-wide shell consistency, deliberately excluded from this follow-up.
