"""Task 2: an invoice shows amounts the way people read them: $1,234.50."""
import unittest
from decimal import Decimal

from tally.invoice import render


class Invoice(unittest.TestCase):
    def test_amounts_read_as_dollars(self):
        text = render("Acme", [("Design", Decimal("1234.5")), ("Hosting", Decimal("99"))])
        self.assertIn("Design: $1,234.50", text)
        self.assertIn("Hosting: $99.00", text)
        self.assertIn("Total: $1,333.50", text)
