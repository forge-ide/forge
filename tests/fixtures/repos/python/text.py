"""Small text helpers."""


def title(name: str) -> str:
    """`name` with its first letter capitalized."""
    return name[:1].upper() + name[1:]
