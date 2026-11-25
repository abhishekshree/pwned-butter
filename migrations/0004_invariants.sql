-- Invariants for new and updated rows only. Legacy history is left untouched:
-- 13 out-of-scope cities and 77 free-text outlet_type values are real rows, not
-- bad data to delete. NOT VALID enforces the checks on writes without demanding
-- that the existing rows satisfy them (which would block the migration).
-- Audit found 0 rows with compliance_score outside 0..100, so that one is validated.
--
-- Clarification of 0003's stale comment (left byte-for-byte unchanged because
-- SQLx checks applied migrations' checksums): db.rs never matched across action
-- types. Dedup is same-action-type only: EventKey over ±10 days in Rust and
-- daterange(d-5, d+5, '[]') in SQL. Postgres canonicalizes that range to
-- [d-5, d+6), so two rows conflict when their dates are up to 10 days apart —
-- the same window db.rs checks with BETWEEN d-10 AND d+10.

ALTER TABLE actions ADD CONSTRAINT actions_compliance_score_range
    CHECK (compliance_score BETWEEN 0 AND 100);

ALTER TABLE actions ADD CONSTRAINT actions_city_scope
    CHECK (city IS NULL OR city IN ('Mumbai','Pune','Thane','Navi Mumbai','Lonavala'))
    NOT VALID;

ALTER TABLE actions ADD CONSTRAINT actions_outlet_type_vocabulary
    CHECK (outlet_type IS NULL OR outlet_type IN ('restaurant','cloud_kitchen','quick_commerce','dhaba','hotel','bakery','club','mess','dairy','street_vendor','other'))
    NOT VALID;

-- One identity policy for the whole pipeline: establishment + city + area +
-- action type, within the ±10 day window above. Two branches of one outlet can
-- share a name, so both the source constraint and the cross-source backstop
-- carry locality. These replacements are strictly weaker than the originals:
-- every pair they rejected is still rejected. NULL city/area group under ''.
ALTER TABLE actions DROP CONSTRAINT actions_source_url_establishment_action_date_key;

-- Postgres does not allow expressions in ALTER TABLE ... ADD CONSTRAINT UNIQUE,
-- so the same key is a unique index. ON CONFLICT in db.rs infers this index.
CREATE UNIQUE INDEX actions_source_identity
    ON actions (source_url, establishment, action_date, coalesce(city, ''), coalesce(area, ''));

ALTER TABLE actions DROP CONSTRAINT actions_no_dupe_events;

ALTER TABLE actions ADD CONSTRAINT actions_no_dupe_events
    EXCLUDE USING gist (
        (lower(establishment)) WITH =,
        (coalesce(lower(city), '')) WITH =,
        (coalesce(lower(area), '')) WITH =,
        action_type WITH =,
        daterange(action_date - 5, action_date + 5, '[]') WITH &&
    );
