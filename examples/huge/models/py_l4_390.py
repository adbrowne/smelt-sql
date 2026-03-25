from smelt import model

@model
def py_l4_390(project):
    """Generated model: union tagged."""
    parts = []
    for dep in ['sql_l3_36']:
        parts.append(f"SELECT user_id, event_time, amount FROM smelt.ref('{dep}')")
    return """
---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
""" + "\nUNION ALL\n".join(parts)
