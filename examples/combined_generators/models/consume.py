from smelt import model


@model
def py_consumer(project):
    """Python model that consumes a SQL generator's emission via combined loop."""
    return "SELECT doubled FROM smelt.emit.enriched"
