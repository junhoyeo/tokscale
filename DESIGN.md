# Design

## Source of truth

- Status: Active
- Last refreshed: 2026-07-12
- Primary product surfaces: Public user profiles at `/u/[username]`, the profile embed dialog, public README widgets at `/api/embed/[username]/svg`, and `/profile` as the authenticated redirect.
- Evidence reviewed: `packages/frontend/src/app/u/[username]/ProfilePageClient.tsx`, `packages/frontend/src/components/profile/`, `packages/frontend/src/lib/embed/`, `packages/frontend/src/app/api/embed/[username]/svg/route.ts`, `packages/frontend/src/components/GraphContainer.tsx`, `packages/frontend/src/components/GraphControls.tsx`, `packages/frontend/src/components/StatsPanel.tsx`, `packages/frontend/src/components/layout/Navigation.tsx`, `packages/frontend/src/app/globals.css`, the current production profile and embed renderers for `junhoyeo`, and the compact content/usage references at `https://cho.sh/ko` and `https://cho.sh/ko/mini/usage`.
- Visual reference captures: `.omx/artifacts/visual-ralph/compact-profile/cho-*-desktop.png`, `.omx/artifacts/visual-ralph/compact-profile/cho-*-mobile.png`, `.omx/artifacts/visual-ralph/compact-profile/current-profile-*.png`, and `.omx/artifacts/embed-redesign/`.

## Brand

- Personality: Precise, technical, calm, and quietly competitive.
- Trust signals: Exact usage values, transparent time ranges, accessible data labels, visible freshness, and familiar GitHub identity.
- Avoid: Cosmic decoration inside application screens, oversized pill controls, repeated metrics, gratuitous 3D treatment, excessive gradients, dense card stacks, and copying another product's branding.

## Product goals

- Goals: Make a profile understandable in one viewport; foreground usage history; make period, metric, and provider controls obvious; make every embed template communicate one distinct usage story at README scale; retain sharing utility; work cleanly from 320px through desktop.
- Non-goals: No database, API, authentication, leaderboard, settings, or `/local` graph redesign in this pass. No new dependency and no pixel-for-pixel clone of the reference sites.
- Success signals: Headline identity and four metrics scan in seconds, the default chart exposes a readable model-level trend without configuration, secondary data is progressively disclosed, and the mobile page has no nested horizontal scrollers.

## Personas and jobs

- Primary personas: A developer reviewing their own activity; a visitor comparing a public profile; a maintainer debugging submitted usage.
- User jobs: Identify the person, understand scale and recency, inspect a time trend, filter by provider, compare token categories/models/devices, configure a truthful embed, and share it in GitHub Markdown or HTML.
- Key contexts of use: Desktop comparison, mobile link preview, GitHub README rendering on narrow and wide containers, keyboard-only navigation, and profiles with long names, many providers, sparse data, or no optional device/model detail.

## Information architecture

- Primary navigation: Existing Tokscale application navigation remains unchanged.
- Core routes/screens: `/u/[username]` is the canonical public profile; `/profile` redirects authenticated users; profile tabs switch between Usage and Models without navigating away; the embed dialog configures the stable `/api/embed/[username]/svg` URL contract without hiding unsupported options.
- Content hierarchy: Identity and freshness → headline metrics → range/tab controls → paired usage and contribution views → persistent selected-day contribution detail → compact supporting insights and token mix → devices.

## Design principles

- Data before decoration: Labels, values, trend, and range context carry the hierarchy.
- One fact, one home: Do not repeat tokens, cost, active time, or sessions across multiple cards.
- Lightest useful surface: Use whitespace and dividers first; reserve bordered panels for the identity overview, charts, Usage details, Token mix, and independently grouped datasets.
- Compact, not cramped: Desktop controls use 28–36px heights; mobile keeps 44–48px coarse-pointer targets without inflating visual chrome.
- Reference, not replica: Adopt the reference's narrow content measure, restrained borders, chart-first composition, and low-noise controls while retaining Tokscale typography, data, and blue accent.
- One widget, one job: Template differences come from information hierarchy and reading density, never costume, decorative metaphor, or renamed identical layouts.
- Tradeoffs: The public profile keeps its purpose-built responsive usage trend and adds an optional inline isometric contribution view using the same scoped calendar as 2D. It does not reuse the heavier `/local` graph container or decorative 3D embed card. Raw totals remain authoritative, the usage trend still defaults to a trailing average, and 2D remains the default contribution view.

## Visual language

- Color: Dark zinc-neutral canvas; raised surfaces only slightly lighter; translucent white borders; white/default/muted text with WCAG AA contrast; Tokscale blue for the single primary action and selected data emphasis; provider colors only in chart/legend context.
- Typography: Existing Figtree UI font and JetBrains Mono for code only. Page title 20–24px medium/semibold, section title 16–18px medium, body 14–16px, metadata 12–13px where it is supplementary rather than body copy. Numeric values use tabular figures.
- Spacing/layout rhythm: 4px base; common gaps 8/12/16/20/24px; profile analytics canvas max-width 1500px with responsive 16–32px gutters and roughly 650px reserved for the desktop contribution column. Keep headline profile metrics left-packed in compact tracks instead of stretching them across the canvas.
- Shape/radius/elevation: 8px controls, 12px panels, full radius only for badges/avatars where semantically appropriate. Dark application surfaces use borders, not shadows.
- Motion: Immediate color/background state changes; 120–160ms transform only for pressed controls; honor `prefers-reduced-motion`.
- Imagery/iconography: GitHub avatar with a subtle dark-surface outline at 72px mobile and 80px desktop. Give rank one compact accent-backed emphasis beside identity metadata. Reuse existing 16px application icons and source assets; avoid decorative icon containers.

## Components

- Existing components to reuse: `Navigation`, `ProfileDevices`, formatters in `lib/utils`, shared graph palettes/settings, source labels/colors, and existing 16px icons.
- New/changed components: Compact `ProfileOverview`, `ProfileTabBar`, profile-only `ProfileUsageChart`, controlled 2D/3D `ProfileContributionGraph`, persistent `ProfileContributionBreakdown`, `ProfileInsights`, `TokenBreakdown`, and semantic models/devices tables.
- Variants and states: Primary/secondary/ghost actions; active/inactive tabs and ranges; all/single-provider chart; tokens/cost metric; trailing-average/daily display; empty and sparse charts; hovered, keyboard-selected, and tapped chart days; selectable contribution palettes; responsive table overflow.
- Chart contract: Render one stable stacked area per provider/model pair. Order provider groups and their models by raw scoped usage ascending so dominant bands remain on top; sort only the active tooltip rows descending. Use provider-level legend colors, deterministic model shades, 40% fills, 1px monotone boundaries, and no chart animation.
- Contribution contract: Render the complete requested UTC date range, including zero-valued outer days; derive 2D intensity and 3D height from the same token-scoped calendar; expose compact view and palette selectors; show a viewport-clamped daily tooltip on hover/focus; and let click, tap, Enter, or Space update one persistent token, cost, client, and model breakdown. Default that breakdown to the visible range end, preserve it across view and palette changes, retain roving keyboard navigation in both views, and expose a concise screen-reader summary.
- Embed contract: Keep the live preview visually primary, place dense settings in a viewport-contained scroll region, expose only options the selected renderer consumes, and trap/restore focus while the dialog is open. The eight 2D templates share one solid surface, identity header, divider, footer, type scale, and restrained semantic colors while using distinct data hierarchies; decorative gradients, glows, patterns, fake chrome, and metaphor-heavy ornament are excluded. The 3D contribution view remains a supported first-class renderer with its own compatible controls. Desktop uses preview/settings panes; mobile uses one body scroll.
- Embed hierarchy: `Overview` balances identity and the three canonical facts; `Token focus` gives tokens dominant scale; `Readout` is a genuinely terse monospace key/value view; `Contributions` makes the calendar the hero; `Rank focus` centers standing and percentile context; `Activity summary` compares measurable one-year activity signals; `Detailed stats` is the densest two-column fact sheet; `Compact list` is the narrowest scan-first ledger. Do not add template-name overlines, invented system labels, or explanatory slogans inside the SVG.
- Token/component ownership: Additive service tokens live in `src/app/globals.css`; profile composition and variants live in `src/components/profile/`. Shared `/local` graph components are out of scope.

## Accessibility

- Target standard: WCAG 2.2 AA.
- Keyboard/focus behavior: Visible focus rings; roving focus for tabs; arrow-key chart day navigation; focusable contribution days; native selects; Tab containment plus Escape/opener focus restoration in the embed dialog.
- Contrast/readability: Muted text must remain at least 4.5:1 for normal text; provider color is never the only data label; body text is at least 16px on mobile.
- Screen-reader semantics: Structured headings, `dl` for metrics, table semantics for models/devices, labeled tabpanels, accessible chart title/description, and a concise live textual readout for committed chart selections. Pointer hover never floods the live region.
- Reduced motion and sensory considerations: Disable nonessential transform/animation under reduced motion; preserve labels and values independently of hue.

## Responsive behavior

- Supported breakpoints/devices: 320px mobile through wide desktop; primary checks at 390, 768, and 1024+ CSS pixels. The usage chart targets 224px height on mobile and 256px on desktop.
- Layout adaptations: Four metric cells become two columns; identity/actions and controls wrap. At 1360px and wider, activity becomes two independent stacks: a left Contributions + complete selected-day breakdown column at roughly 650px, and a still-wider right Usage over time + Usage details + Token mix column. Contribution cards keep intrinsic height, and the standalone breakdown expands with the full client/model list instead of using an internal scroll region. Tablet and mobile collapse to Contributions → selected-day breakdown → Usage → Usage details → Token mix → Devices. The area chart, seven-row 2D calendar, and range-sized isometric SVG compress within their containers without page overflow; tables expose secondary detail instead of silently dropping it. Embed SVGs retain their public intrinsic widths while scaling down proportionally in README and modal containers; the modal changes from two panes to one document scroll below 840px.
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
- Performance constraints: Keep chart and contribution geometry derivation memoized; allow normal profiles to retain their model bands, apply a high pathological series cap with an explicit remainder, render only one contribution view at a time, and preserve server data fetching and ISR.
- Analytical constraints: Missing calendar dates are zero-valued. Lifetime defaults to a trailing 30-day average and finite ranges to a trailing 7-day average, with daily values available as an explicit display mode. Moving averages never alter raw range totals or stable series ranking.
- Compatibility constraints: Leave API/auth/database contracts and canonical profile redirects unchanged. Do not mutate the shared `GraphContainer` behavior. Preserve all public embed template IDs and query parameters, XML escaping, CSP-compatible standalone SVG output, intrinsic template widths, and the classic fallback for invalid or omitted templates.
- Test/screenshot expectations: Unit-test chart aggregation, contribution selection, isometric geometry, embed parsing/dispatch, shared SVG structure, bounds, escaping, and formatting; run frontend tests/lint/typecheck/build; capture `/u/junhoyeo` locally in 2D and 3D at 1440×1100 and 390×844; capture every embed template with actual data in dark and light themes plus representative graph and compact states; persist each visual verdict under `.omx/state/` with a pass target of 90.

## Open questions

- [ ] Decide in a future pass whether the compact service language should extend to leaderboard, groups, settings, navigation, and footer; owner: product/design; impact: global shell consistency, deliberately excluded from this profile-focused branch.
