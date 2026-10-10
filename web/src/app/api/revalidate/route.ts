import { revalidateTag } from "next/cache";
import { NextResponse } from "next/server";

export const dynamic = "force-dynamic";

const TAGS = ["actions"];

// POST /api/revalidate  { "secret": "...", "tags": ["actions"]? }
// Clears the dashboard's cached queries so fresh rows show immediately.
// The CLI in src/bin/cache_bust.rs calls this after a backfill or fix.
export async function POST(req: Request) {
  const expected = process.env.REVALIDATE_SECRET;
  if (!expected) {
    return NextResponse.json(
      { error: "REVALIDATE_SECRET is not configured" },
      { status: 500 },
    );
  }
  let body: { secret?: unknown; tags?: unknown };
  try {
    body = (await req.json()) as typeof body;
  } catch {
    return NextResponse.json({ error: "invalid JSON body" }, { status: 400 });
  }
  if (body.secret !== expected) {
    return NextResponse.json({ error: "bad secret" }, { status: 401 });
  }
  const tags = Array.isArray(body.tags)
    ? body.tags.filter((t): t is string => typeof t === "string" && t.length > 0)
    : TAGS;
  const revalidated = [...new Set([...TAGS, ...tags])].filter((t) =>
    TAGS.includes(t),
  );
  for (const tag of revalidated) revalidateTag(tag, "max");
  return NextResponse.json({ revalidated });
}
