-- One enforcement event per establishment per action type within ±5 days.
-- Re-reports by later outlets previously created duplicate cards.

-- Self-heal any pairs that would block the constraint (keeps earliest id).
DELETE FROM actions a USING actions b
WHERE a.id > b.id
  AND lower(a.establishment) = lower(b.establishment)
  AND a.action_type = b.action_type
  AND a.action_date BETWEEN b.action_date - 5 AND b.action_date + 5;

CREATE EXTENSION IF NOT EXISTS btree_gist;

ALTER TABLE actions ADD CONSTRAINT actions_no_dupe_events
    EXCLUDE USING gist (
        (lower(establishment)) WITH =,
        action_type WITH =,
        daterange(action_date - 5, action_date + 5, '[]') WITH &&
    );
