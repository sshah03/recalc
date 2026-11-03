from decimal import Decimal


def total(items):
    """Sum of quantity times unit price, to the cent."""
    return sum((Decimal(str(q)) * Decimal(str(p)) for q, p in items), Decimal("0")).quantize(Decimal("0.01"))
