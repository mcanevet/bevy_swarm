# 0002: Dual MIT/Apache-2.0 License, No CLA, Inbound=Outbound

Date: 2026-10-14

## Status

Accepted.

## Context

- bevy_playtest is an ECS-native playtesting harness for Bevy games.
- Bevy itself uses dual MIT/Apache-2.0 licensing.
- The Rust ecosystem standard is dual licensing without a Contributor License Agreement (CLA).
- The project owner (Mickaël Canévet) intends to keep licensing stable and contributor-friendly.

## Decision

- **License**: Dual MIT OR Apache-2.0 (at the option of downstream users).
- **Copyright**: LICENSE-MIT includes "Copyright (c) 2026 Mickaël Canévet".
- **Contributions**: Standard Rust contribution clause — "Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions."
- **No CLA**: Contributors retain copyright; no assignment required.
- **Sponsorship**: Deferred until the user enrolls in GitHub Sponsors (FUNDING.yml removed).

## Consequences

- Downstream projects can choose either license.
- Contributors retain copyright and grant non-exclusive rights under the dual license.
- No legal overhead from CLAs.
- Sponsorship recognition deferred to avoid premature commitments.
