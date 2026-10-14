# Reference: Invariant DSL

The **JSON invariant language** in scenarios is the ONE supported way
to express checks (`check`, `value`, `query`, `path`, windows
`after_s`/`before_s`/`eventually_s`). It parses into one `CheckOp`
enum everywhere: wire format `lt`, `le`, `gt`, `ge`, `equals`, `ne`
(legacy aliases `below` = `le`, `above` = `ge`, `eq` = `equals`; all
inclusive boundaries). Calibration output
(`generate_invariants_from_calibration`) emits exactly this dialect —
round-trip guaranteed by test.

Canonical wire forms (what `docs/schema.json` accepts):

| Op | Meaning | Alias |
|---|---|---|
| `lt` | strictly less than | — |
| `le` | at most | `below` |
| `gt` | strictly greater than | — |
| `ge` | at least | `above` |
| `equals` | equality (numeric or string) | `eq` |
| `ne` | inequality | — |

For how to compose invariants, see the
[how-to guide](howto-add-invariant.md).

`bevy_swarm::typed` (Rust-native oracle/predicate/policy systems) is
**experimental**: usable, but expect breaking changes without a
major version bump; the JSON DSL is the stable surface for v0.1.
