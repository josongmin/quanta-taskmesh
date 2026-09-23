"""Reviewed resource policy for bounded parallel verification gates."""

PARALLEL_GATE_LIMIT = 4

PARALLEL_GROUP_MEMBERS = {
    "static-independent": frozenset(
        {
            "fmt-check",
            "gates-inventory",
            "py-lint",
            "test-architecture",
            "semgrep",
            "deny",
        }
    )
}
