# Fixture: Walker

**Clean**: player moves in response to Move intents.  
**Buggy** (`FIXTURE_BUG=desync`): input wiring broken — player never moves.

## Controls

None (driven by test harness intents).

## Planted Bugs

| Bug | FIXTURE_BUG | Behavior |
|-----|-------------|----------|
| desync | `desync` | Input wiring broken |
