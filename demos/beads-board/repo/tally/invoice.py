from .money import fmt


def render(customer, lines):
    """lines: [(description, amount)]. Returns the invoice as text."""
    out = [f"Invoice for {customer}"]
    total = 0
    for desc, amount in lines:
        out.append(f"  {desc}: {fmt(amount)}")
        total += amount
    out.append(f"  Total: {fmt(total)}")
    return "\n".join(out)
