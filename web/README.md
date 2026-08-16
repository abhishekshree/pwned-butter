# Dashboard (web)

Next.js 16 + Tailwind v4 + shadcn/ui (Base UI) + Recharts dashboard that reads
the Neon database directly over Neon's HTTP serverless driver — no API
functions, no pooled TCP connections.

## Run locally

```bash
cp ../.env .env.local      # for the DATABASE_URL var (or paste one)
pnpm install
pnpm dev
```

Open http://localhost:3000. The page is server-rendered and URL-driven: every
filter combination (`/?brand=…&city=…&status=…`) is a shareable link.

## Checks

```bash
pnpm lint
pnpm exec tsc --noEmit
pnpm build
```

## Notes

- Data is written by the Rust ETL in the repo root (see the root README); this
  app is strictly read-only.
- Package manager is pnpm; `pnpm-lock.yaml` is the lockfile. If pnpm flags
  ignored build scripts, run `pnpm approve-builds`.