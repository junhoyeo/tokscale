import { companiesCsv } from "@/lib/capital/ledger/csv";

/**
 * The companies directory as CSV. Same facts as the page; filtered views are not exported.
 * Not for search engines: the HTML directory is the canonical page.
 */
export const dynamic = "force-static";

export function GET() {
  return new Response(companiesCsv(), {
    headers: {
      "Content-Type": "text/csv; charset=utf-8",
      "Content-Disposition": 'attachment; filename="tokscale-capital-companies.csv"',
      "X-Robots-Tag": "noindex",
    },
  });
}
