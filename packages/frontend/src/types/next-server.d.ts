declare module "next/server" {
  export type NextRequest = Request & {
    nextUrl: URL;
    cookies: {
      get(name: string): { value: string } | undefined;
    };
  };

  export class NextResponse extends Response {
    constructor(body?: BodyInit | null, init?: ResponseInit);

    static json(body?: unknown, init?: ResponseInit): NextResponse;
    static next(init?: ResponseInit): NextResponse;
    static redirect(url: string | URL, init?: number | ResponseInit): NextResponse;
  }
}
