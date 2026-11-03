"""Task 3: the accounting export gains a tax column at 8%, and its amounts
stay plain numbers the accounting system can read."""
import unittest
from decimal import Decimal

from tally.export import csv_text


class Export(unittest.TestCase):
    def test_tax_column_and_plain_numbers(self):
        out = csv_text([("Design", Decimal("1234.5")), ("Hosting", Decimal("99"))])
        self.assertEqual(
            out,
            "description,amount,tax\nDesign,1234.50,98.76\nHosting,99.00,7.92\n",
        )
