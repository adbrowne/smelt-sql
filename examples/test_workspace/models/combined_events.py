from smelt import model


@model
def combined_events(project):
    """Combine all event source models into a single UNION ALL."""
    children = project.find_models(tag="event_source")
    if not children:
        # Fallback: just select from raw_events (path form, Phase 4)
        return "SELECT event_id, user_id, event_time, event_type FROM smelt.models.raw_events"
    refs = [f"SELECT * FROM smelt.models.{m.name}" for m in children]
    return " UNION ALL ".join(refs)
