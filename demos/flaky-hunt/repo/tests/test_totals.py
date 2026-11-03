import unittest
from decimal import Decimal

from shipit.totals import total


class TotalsTest(unittest.TestCase):
    def test_line_items_total_to_the_cent(self):
        self.assertEqual(total([(3, 19.99), (1, 0.1), (2, 0.2)]), Decimal("60.47"))
