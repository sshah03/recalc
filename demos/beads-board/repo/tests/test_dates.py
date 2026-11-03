"""Task 4: a due date that lands on a weekend moves to the next Monday."""
import datetime as dt
import unittest

from tally.dates import due


class Due(unittest.TestCase):
    def test_weekday_unchanged(self):
        self.assertEqual(due(dt.date(2026, 9, 1)), dt.date(2026, 10, 1))  # a Thursday

    def test_weekend_moves_to_monday(self):
        self.assertEqual(due(dt.date(2026, 10, 1)), dt.date(2026, 11, 2))  # Oct 31 is a Saturday
        self.assertEqual(due(dt.date(2026, 10, 2)), dt.date(2026, 11, 2))  # Nov 1 is a Sunday
