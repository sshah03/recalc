import datetime as dt


def due(issued, days=30):
    """The due date: `days` calendar days after issue."""
    return issued + dt.timedelta(days=days)
