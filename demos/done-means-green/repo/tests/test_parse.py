"""Task 1: amounts typed by people carry thousands separators."""
import unittest
from decimal import Decimal

from tally.money import parse


class Parse(unittest.TestCase):
    def test_plain(self):
        self.assertEqual(parse("1234.50"), Decimal("1234.50"))

    def test_thousands_separators(self):
        self.assertEqual(parse("1,234.50"), Decimal("1234.50"))
        self.assertEqual(parse("12,345,678"), Decimal("12345678.00"))
