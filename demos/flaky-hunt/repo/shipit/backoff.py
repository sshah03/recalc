import random
import time

BASE = 0.01


def wait(attempt):
    """Sleep before retry number `attempt` (0-based): a base delay plus jitter."""
    delay = BASE * (2 ** attempt) + random.uniform(0, 0.02)
    time.sleep(delay)
    return delay
