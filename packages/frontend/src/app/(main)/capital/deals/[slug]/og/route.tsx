import { ledger } from "@/lib/capital/ledger";
import { eventOgImage } from "../../../_components/eventOg";

/** 1200x630 Open Graph card for this page (see _components/eventOg.tsx). */
export const dynamic = "force-dynamic";

export async function GET(_request: Request, { params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  const e = ledger.getEvent(slug);
  if (!e || e.route !== "deals") return new Response("Not found", { status: 404 });
  return eventOgImage(e);
}
