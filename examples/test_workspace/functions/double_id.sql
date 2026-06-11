-- Phase 5 (nullability-soundness): fixture for NOT NULL parameter checking.
-- `event_id` in the source has `nullable: false`; passing it here is clean.
smelt.define double_id(id: Expr<Integer NOT NULL>) -> Expr<Integer>
    AS (id * 2)
