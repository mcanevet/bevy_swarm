# Reference: Robustness Margins (J1)

Every invariant carries a **signed robustness ρ** — the STL-style
distance to violation: positive when satisfied ("never violated, but
came within 0.3 of the bound"), negative when violated.
`report.robustness` holds:

- `per_invariant`: for each invariant, `(min ρ over run, frame of
  worst, normalized min)`
- `overall`: min of normalized mins across all invariants
  (< 0 iff the run failed)

Units normalize via an optional `scale` field (default
`max(|threshold|, 1.0)`; bounds use the half-width) so pixel-scale
bounds don't swamp HP-scale ones.

**Semantics by invariant kind:**

| Invariant | Robustness ρ |
|-----------|-------------|
| `below`/`le` thr | `thr - v` |
| `above`/`ge` thr | `v - thr` |
| `lt`/`gt` | same as `le`/`ge` (strictness matters at ρ = …) |
| `equals` thr | `-(distance)` (0 when exact) |
| text `equals` | `+1` if equal, else `-1` (boolean lift) |
| `bounds` (per entity) | `min over axes of min(v - lo, hi - v)`; aggregated `min` over entities |
| `count` query | threshold margin on the count |
| `no_decrease` | `v_t - v_{t-1}` (min over run) |
| `max_delta_per_sec` | `max_dps - rate` |
| `eventually` (deadline D) | `max over t≤D of ρ_inner(t)` (best value so far) |
| expert `WHEN→REQUIRE` | `max(-ρ_W, ρ_R)` (anti-masking) |
| always-mode (default) | run-level ρ = `min over t of ρ(t)` |

Boolean verdict is exactly `ρ < 0` (or `ρ ≤ 0` for strict ops); this
equivalence is enforced by `debug_assert!` in the oracle so
semantics can't drift.

```json
{"invariants": [{"name": "hp_floor", "rule": "custom", "path": "TestApi.hp",
                 "check": "above", "value": 10, "scale": 100}]}
```

This continuous signal is the substrate for future guided search
(choice mutation, Go-Explore) and LLM falsifiers.
