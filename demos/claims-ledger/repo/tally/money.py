"""Amounts are Decimals, rounded to cents."""
from decimal import Decimal, ROUND_HALF_UP

CENT = Decimal("0.01")


def parse(text):
    """'1234.50' -> Decimal('1234.50')."""
    return Decimal(text.strip()).quantize(CENT, rounding=ROUND_HALF_UP)


def fmt(amount):
    """Decimal('1234.5') -> '1234.50'."""
    return str(Decimal(amount).quantize(CENT, rounding=ROUND_HALF_UP))
