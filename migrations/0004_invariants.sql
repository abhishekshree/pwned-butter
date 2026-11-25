-- Invariants for new and updated rows only. Legacy history is left untouched:
-- 13 out-of-scope cities and 77 free-text outlet_type values are real rows, not
-- bad data to delete. NOT VALID enforces the checks on writes without demanding
-- that the existing rows satisfy them (which would block the migration).
-- Audit found 0 rows with compliance_score outside 0..100, so that one is validated.
--
-- Clarification of 0003's stale comment (left byte-for-byte unchanged because
-- SQLx checks applied migrations' checksums): db.rs never matched across action
-- types. Dedup is same-action-type only: EventKey over ±10 days in Rust, with
-- the SQL EXCLUDE backstop at ±5 days.

ALTER TABLE actions ADD CONSTRAINT actions_compliance_score_range
    CHECK (compliance_score BETWEEN 0 AND 100);

ALTER TABLE actions ADD CONSTRAINT actions_city_scope
    CHECK (city IS NULL OR city IN ('Mumbai','Pune','Thane','Navi Mumbai','Lonavala'))
    NOT VALID;

ALTER TABLE actions ADD CONSTRAINT actions_outlet_type_vocabulary
    CHECK (outlet_type IS NULL OR outlet_type IN ('restaurant','cloud_kitchen','quick_commerce','dhaba','hotel','bakery','club','mess','dairy','street_vendor','other'))
    NOT VALID;

-- Event identity includes the city (two branches can share a name), so the
-- exclusion backstop must too. This is strictly weaker than the old constraint:
-- every pair it previously rejected is still rejected. NULL cities group under
-- '' so legacy rows cannot duplicate each other.
ALTER TABLE actions DROP CONSTRAINT actions_no_dupe_events;

ALTER TABLE actions ADD CONSTRAINT actions_no_dupe_events
    EXCLUDE USING gist (
        (lower(establishment)) WITH =,
        (coalesce(lower(city), '')) WITH =,
        action_type WITH =,
        daterange(action_date - 5, action_date + 5, '[]') WITH &&
    );
