-- Collapse MCA BKC re-reports into the canonical x.com suspension row.
-- Name variants ("MCA BKC Club") and action-type drift ("inspection" vs
-- "licence_suspension") slipped past exact-match dedup; db.rs now matches
-- acronyms across action types within ±5 days, this heals existing rows.

DELETE FROM actions WHERE lower(establishment) = 'mca bkc club';

DELETE FROM actions
WHERE action_type = 'inspection'
  AND establishment ILIKE '%cricket%association%'
  AND establishment ILIKE '%bkc%';
