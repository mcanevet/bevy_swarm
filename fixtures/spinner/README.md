# Fixture: Spinner

**Clean**: entities rotate continuously.  
**Buggy** (`FIXTURE_BUG=frozen`): rotation stalls after frame 30 (soft-lock).

## Controls

None (automatic rotation).

## Planted Bugs

| Bug | FIXTURE_BUG | Behavior |
|-----|-------------|----------|
| frozen | `frozen` | Rotation stops after frame 30 |

## Adding a Fixture

See `docs/conventions.md` §3.
