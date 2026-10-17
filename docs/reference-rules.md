# Rule Catalog (Reference)

The authoritative per-rule reference is the generated catalog:

→ **[rules.md](rules.md)** — generated from `src/rules.rs` (`Rule`
enum); regenerate with
`BEVY_PLAYTEST_UPDATE=schema cargo test --test docs_generation`.

Every rule documents: what it detects, false-positive modes, and its
kind (`major` = gates run status, `informational` = reported only).

Related reference pages:

- [Schema reference](schema.json) — the scenario JSON contract
- [Contract tiers](reference-contract-tiers.md) — detection matrix
- [Report schema](report-schema.json) — the output contract

For how to react to a specific rule firing, see
[debug a failing run](howto-debug-failure.md) and
[suppress known failures](howto-suppress-known-failure.md).
