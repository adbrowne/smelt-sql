-- Users LEFT JOINed with their latest event.
--
-- Fixture for outer-join nullability soundness tests (spec §11):
-- `event_id` is declared nullable: false in sources/source/events.yml,
-- but appears on the null-supplying right side of a LEFT JOIN, so it must
-- infer as nullable: true in this model's output schema.
SELECT
    u.user_id,
    e.event_id,
    e.event_type
FROM smelt.users AS u
LEFT JOIN smelt.sources.source.events AS e
    ON u.user_id = e.user_id
