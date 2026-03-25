from smelt import model

@model
def py_l3_453(project):
    """Generated model: union tagged."""
    parts = []
    for dep in ['py_l2_476']:
        parts.append(f"SELECT user_id, event_time, amount FROM smelt.ref('{dep}')")
    return """
---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
""" + "\nUNION ALL\n".join(parts)
