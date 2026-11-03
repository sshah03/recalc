import time
import unittest

from shipit import backoff


class BackoffTest(unittest.TestCase):
    def test_the_first_retry_waits_under_28ms(self):
        start = time.monotonic()
        backoff.wait(0)
        self.assertLess(time.monotonic() - start, 0.028)
