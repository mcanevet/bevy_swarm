# turn_based

Card game vs AI. Enter ends the turn. Score increments as cards are played.

Runs as a plain game: `cargo run -p fixture_turn_based`.

## Bug variants (set `FIXTURE_BUG`)

| FIXTURE_BUG       | Bug                          | Expected                  |
|--------------------|------------------------------|---------------------------|
| checkop_boundary   | score forced to exactly 50.0 | custom invariant boundary |
| (unset)            | none                         | pass                      |
