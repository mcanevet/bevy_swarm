# Fixture: Spawner

**Clean**: spawns exactly 10 entities, then stops.  
**Buggy** (`FIXTURE_BUG=leak`): spawns every frame (memory leak).

## Planted Bugs

| Bug | FIXTURE_BUG | Behavior |
|-----|-------------|----------|
| leak | `leak` | Unbounded spawning |
