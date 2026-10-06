# regression_pair

One game used for golden-divergence testing. FIXTURE_BUG selects the "version":

| FIXTURE_BUG    | Change                        | Expected                     |
|----------------|-------------------------------|------------------------------|
| v2_damage      | damage formula 10.0 → 15.0    | golden_diverged (Z11, pending) |
| v2_slow_menu   | menu animation slower         | must NOT diverge (T11, pending) |
| (unset)        | baseline                      | pass                         |
