import { neon } from "@neondatabase/serverless";

type Sql = ReturnType<typeof neon>;

let client: Sql | null = null;

// Lazy connection: importing this module must not require a DATABASE_URL;
// it is only needed when a query actually runs.
export function sql(text: string, params?: unknown[]) {
  if (!client) {
    const url = process.env.DATABASE_URL;
    if (!url) throw new Error("DATABASE_URL is not set");
    client = neon(url);
  }
  return client.query(text, params);
}