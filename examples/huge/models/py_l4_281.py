from smelt import model

@model
def py_l4_281(project):
    """Generated model: conditional SQL."""
    threshold = 57
    if threshold > 50:
        filter_clause = "WHERE amount > 100"
    else:
        filter_clause = "WHERE amount > 0"
    return f"""
---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    event_time,
    amount,
    category
FROM smelt.ref('sql_l3_105')
{filter_clause}
"""
