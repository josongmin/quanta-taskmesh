"""The preregistration file must fail closed before a performance claim."""

import json
from pathlib import Path

CONTRACT = Path(__file__).resolve().parents[1] / "scenarios" / "claim-contract.json"


def test_unmeasured_claims_and_peer_population_remain_blocked() -> None:
    contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
    assert contract["schema_version"] == 1
    assert contract["status"].startswith("blocked_")
    claims = {claim["id"]: claim for claim in contract["claims"]}
    assert set(claims) == {
        "governor-named-operation-cost",
        "public-host-slo-capacity",
        "named-industry-peer-comparison",
    }
    assert all(claim["status"].startswith("blocked_") for claim in claims.values())
    assert all(claim["primary_metric"] and claim["denominator"] for claim in claims.values())
    assert all(claim["denial_conditions"] for claim in claims.values())
    assert claims["public-host-slo-capacity"]["slo_ms"] is None
    assert claims["public-host-slo-capacity"]["minimum_completion_fraction"] is None
    design = contract["study_design"]
    assert design["rate_grid"]["status"] == "unfrozen_pending_pilot"
    assert design["rate_grid"]["absolute_rates_per_second"] == []
    assert design["minimum_detectable_effect_fraction"] is None
    assert design["run_policy"]["repetitions_per_rate"] is None
    assert design["run_policy"]["exclusions"]
    assert contract["comparators"]["eligible_industry_peers"] == []
    assert len(contract["comparators"]["mechanism_controls"]) == 3
    assert contract["consumer_profile"]["status"] == "blocked_missing_provenance"
