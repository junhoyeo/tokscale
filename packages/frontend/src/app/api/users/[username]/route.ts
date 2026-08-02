import { getPublicProfileResponse } from "@/lib/publicProfileData";

export const revalidate = 60;

interface RouteParams {
  params: Promise<{ username: string }>;
}

export async function GET(request: Request, context: RouteParams) {
  return getPublicProfileResponse(request, context);
}
