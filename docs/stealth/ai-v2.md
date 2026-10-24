# Stealth: tokscale.ai v2 prototype (local only, never pushed)

Branch `stealth/ai-v2` (integration). Workers branch from it in their own worktrees and commit locally. No `git push`, no PRs.

Direction: tokscale.ai becomes the public place to see how people build with AI. Humans and Models are the core; Capital is an AI-economy research section; the CLI story moves to /tracker. tokscale.io (teams) is linked once, near the footer.

## Workstreams and ownership
| Worker | Owns (only edit these unless agreed) |
|---|---|
| data | `src/lib/explore/**` implementation (keep `types.ts` signatures), DB rollups, seed script under `scripts/stealth/`, drizzle migration if needed |
| home | `src/app/(main)/page.tsx`, `src/components/home/**`, `src/app/(main)/tracker/**`, `src/components/layout/Navigation.tsx`, new footer, design tokens for v2 (`src/app/globals.css` additions), new art in `public/assets/v2/` |
| boards | `src/app/(main)/leaderboard/**` (Humans/Models/Agents tabs), `src/app/(main)/models/**`, `src/components/explore/**` |
| capital | `src/app/(main)/capital/**` (company pages at `capital/companies/[slug]`), `src/content/capital/**`, `src/lib/capital/**`, capital sitemap entries in a separate function |

Shared contract: `src/lib/explore/types.ts`. Change it only by asking the parent.

## Design language (v2)
- Centered, quiet hero; serif display headlines; sans UI (Figtree); tabular numbers.
- Thin 1px rules and aligned grids instead of heavy cards/shadows; generous whitespace.
- Monochrome engraving-style art (observatory / instruments) only as backdrop/frame, never beside the hero title.
- Black canvas, one accent blue (#0073FF); cyan only for numbers that need emphasis.
- Every number shows its period and data time; cost is labelled "Est. cost".
- No claims Tokscale cannot back (no skill scores, no "LIVE" unless live). A wall of company logos appears only in the home "Trusted by professionals at" row (owner decision, 2026-10-21; the logos come from community reports). Small identification marks next to a model or company name (vendored LobeHub Icons, MIT, in public/assets/v2/models/) are fine on model, leaderboard and Capital pages; companies without a mark use a monogram.
