from smelt import model


@model
def active_users(project):
    """Users with recent activity."""
    return "SELECT user_id, COUNT(*) as activity_count FROM smelt.ref('user_sessions') WHERE session_count > 0 GROUP BY user_id"


@model
def inactive_users(project):
    """Users with no recent activity."""
    return "SELECT user_id FROM smelt.ref('user_stats') WHERE total_sessions = 0"
