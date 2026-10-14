# Reference: Security (agent feature)

The `agent` feature exposes BRP (arbitrary world reads and
**mutation**) over HTTP. Threat model: **the agent is an open
debugger gated by a shared secret, safe on loopback only**. Remote
use additionally requires a TLS-terminating proxy — the HTTP
transport is plaintext.

- Binds to `127.0.0.1:15702` by default; non-loopback addresses are
  refused unless `AgentConfig { allow_remote: true, .. }` is set
  explicitly.
- **Default-deny**: a token is ALWAYS required for `playtest/*`
  methods. Configure one via `AgentConfig::token` or the
  `BEVY_SWARM_AGENT_TOKEN` env var. When neither is set, a
  cryptographically random session token is generated and printed
  to stderr at startup (eprintln, so it survives even when the
  plugin is added before `LogPlugin`).
- Token comparison is constant-time; an explicitly-empty configured
  token is treated as unset and does not authenticate `{token: ""}`.
- Release builds with the agent active log a loud warning; set
  `deny_in_release: true` to panic instead. Never ship
  agent-enabled builds.

For the design context (why this exists alongside Bevy's own E2E
testing goal), see the
[agent-vs-bevy-e2e explanation](explanation-agent-vs-bevy-e2e.md).
