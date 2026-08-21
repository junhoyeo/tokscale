import { getPublicProfileDevicesResponse } from "@/lib/publicProfileDevices";

export const revalidate = 60;

interface RouteParams {
  params: Promise<{ username: string }>;
}

export async function GET(request: Request, context: RouteParams) {
  return getPublicProfileDevicesResponse(request, context);
}
