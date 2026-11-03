import random


def pick_reviewer(author, team):
    """Someone on the team to review the author's change."""
    return random.choice(team)
