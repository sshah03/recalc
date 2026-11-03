def render(labels):
    """The labels as one line, for the release notes."""
    return ", ".join(set(labels))
