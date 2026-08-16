import { neon } from "@neondatabase/serverless";

type Sql = ReturnType<typeof neon>;

let client: Sql | null = null;

function getClient(): Sql {
  if (!client) {
    const connectionString = process.env.DATABASE_URL;
    if (!connectionString) throw new Error("DATABASE_URL is not set");
    client = neon(connectionString);
  }
  return client;
}

// Lazy connection: importing this module must not require a DATABASE_URL;
// it is only needed when a query actually runs.
export const sql = new Proxy({} as Sql, {
  get(_target, prop) {
    const c = getClient();
    const value = Reflect.get(c, prop, c);
    return typeof value === "function" ? value.bind(c) : value;
  },
});