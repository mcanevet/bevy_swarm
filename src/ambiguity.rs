//! swarm-b6t: ambiguity gate v2 — structured conflict extraction +
//! [`AllowRule`] escape hatch.
//!
//! Replaces the string-parsing gate: conflicts are read from
//! `ScheduleGraph::conflicting_systems()` as `(name_a, name_b,
//! conflict-data)` triples, with the world-exclusive-only pairs
//! filtered by inspecting the conflict data (empty conflict list ==
//! `World` access only), then matched against the scenario's
//! `allow_ambiguities` rules. Anything not allowed rejects.

use bevy::ecs::schedule::LogLevel;
use bevy::ecs::schedule::ScheduleBuildSettings;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// One allowed ambiguity, matched against extracted conflicts.
///
/// Patterns support exact names, `crate::module::` prefixes (trailing
/// `::` matches any system in that module/crate), and `::suffix`
/// matching (leading `::` matches any system whose name ends with the
/// suffix).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AllowRule {
    /// Allow a specific pair of systems (matched in either order).
    Pair {
        /// First system name or pattern.
        a: String,
        /// Second system name or pattern.
        b: String,
    },
    /// Allow all conflicts involving one system.
    System {
        /// System name or pattern.
        name: String,
    },
    /// Allow all conflicts where BOTH systems belong to the given
    /// crate/module prefix (library-internal ambiguity).
    Among {
        /// Crate or module prefix (e.g. `"bevy_ui::"`).
        prefix: String,
    },
    /// Allow all conflicts on a data type (component/resource name),
    /// regardless of the systems involved. Pairs conflicting only on
    /// exclusive `World` access are never allowed this way (they are
    /// already always permitted).
    Data {
        /// Data type name, e.g. `"Transform"`.
        name: String,
    },
}

/// A single extracted conflict (structured, no string parsing of
/// rendered reports).
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub schedule: String,
    pub a: String,
    pub b: String,
    /// Conflict data type names; empty means World-exclusive-only
    /// (already harmless — serialized at execution time).
    pub data: Vec<String>,
}

impl Conflict {
    /// World-exclusive-only conflicts carry no ordering hazard.
    pub fn is_world_only(&self) -> bool {
        self.data.is_empty()
    }
}

fn matches(pattern: &str, name: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("::") {
        // Match any system inside the crate/module (prefix followed
        // by a path separator), not systems whose name merely shares
        // the prefix string (bevy_ui:: vs bevy_uix::).
        return name.starts_with(pattern) || name == prefix;
    }
    if let Some(suffix) = pattern.strip_prefix("::") {
        return name.ends_with(suffix);
    }
    pattern == name
}

fn belongs_to(prefix: &str, name: &str) -> bool {
    matches(&format!("{prefix}::"), name)
}

impl AllowRule {
    /// Does this rule allow the given conflict?
    pub fn allows(&self, c: &Conflict) -> bool {
        match self {
            AllowRule::Pair { a, b } => {
                (matches(a, &c.a) && matches(b, &c.b)) || (matches(a, &c.b) && matches(b, &c.a))
            }
            AllowRule::System { name } => matches(name, &c.a) || matches(name, &c.b),
            AllowRule::Among { prefix } => belongs_to(prefix, &c.a) && belongs_to(prefix, &c.b),
            AllowRule::Data { name } => c.data.iter().any(|d| matches(name, d)),
        }
    }
}

/// Extract all data-conflicts from every schedule in the app
/// (excluding world-only pairs), regardless of `allow` rules.
pub fn extract_conflicts(world: &mut World) -> Vec<Conflict> {
    let mut out = Vec::new();
    let Some(mut schedules) = world.remove_resource::<Schedules>() else {
        return out;
    };
    for (label, sched) in schedules.iter_mut() {
        // Warn-level: initialize() records conflicts in the graph
        // without failing; we read them structurally. Other build
        // errors (cycles etc.) panic later in Schedule::run anyway.
        sched.initialize(world);
        let graph = sched.graph();
        {
            let components = world.components();
            for (a, b, data) in graph.conflicting_systems().iter() {
                let name_a = graph
                    .get_node_name(&bevy::ecs::schedule::NodeId::System(*a))
                    .to_string();
                let name_b = graph
                    .get_node_name(&bevy::ecs::schedule::NodeId::System(*b))
                    .to_string();
                let data: Vec<String> = data
                    .iter()
                    .map(|id| {
                        components
                            .get_name(*id)
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| format!("{id:?}"))
                    })
                    .collect();
                out.push(Conflict {
                    schedule: format!("{label:?}"),
                    a: name_a,
                    b: name_b,
                    data,
                });
            }
        }
    }
    world.insert_resource(schedules);
    out
}

/// The v2 gate: configure schedules for ambiguity detection, force
/// initialization, extract conflicts structurally, apply world-only
/// filtering plus the scenario allow list, reject on the remainder.
/// Returns the human-readable rejection or None when clean.
pub fn gate(app: &mut App, allows: &[AllowRule]) -> Option<String> {
    app.configure_schedules(ScheduleBuildSettings {
        ambiguity_detection: LogLevel::Error,
        ..Default::default()
    });
    let conflicts = extract_conflicts(app.world_mut());
    let rejected: Vec<String> = conflicts
        .into_iter()
        .filter(|c| !c.is_world_only())
        .filter(|c| !allows.iter().any(|r| r.allows(c)))
        .map(|c| {
            format!(
                "schedule {}: `{}` and `{}` conflict on [{}]",
                c.schedule,
                c.a,
                c.b,
                c.data.join(", ")
            )
        })
        .collect();
    if rejected.is_empty() {
        None
    } else {
        Some(format!(
            "schedule ambiguities ({}): {}",
            rejected.len(),
            rejected.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflict(a: &str, b: &str, data: &[&str]) -> Conflict {
        Conflict {
            schedule: "Update".into(),
            a: a.into(),
            b: b.into(),
            data: data.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn pattern_matching_exact_prefix_suffix() {
        assert!(matches("my_game::ai::tick", "my_game::ai::tick"));
        assert!(matches("bevy_ui::", "bevy_ui::layout::foo"));
        assert!(matches("::foo_system", "my_game::bar::foo_system"));
        assert!(!matches("bevy_ui::", "bevy_uix::layout::foo"));
        assert!(!matches("foo", "foobar"));
    }

    #[test]
    fn pair_rule_matches_both_orders() {
        let c = conflict("sys_a", "sys_b", &["Transform"]);
        assert!(AllowRule::Pair {
            a: "sys_a".into(),
            b: "sys_b".into()
        }
        .allows(&c));
        assert!(AllowRule::Pair {
            a: "sys_b".into(),
            b: "sys_a".into()
        }
        .allows(&c));
        assert!(!AllowRule::Pair {
            a: "sys_a".into(),
            b: "sys_c".into()
        }
        .allows(&c));
    }

    #[test]
    fn among_requires_both_sides() {
        let internal = conflict("bevy_ui::a", "bevy_ui::b", &["Node"]);
        let mixed = conflict("bevy_ui::a", "my_game::b", &["Node"]);
        let among = AllowRule::Among {
            prefix: "bevy_ui".into(),
        };
        assert!(among.allows(&internal));
        assert!(!among.allows(&mixed));
    }

    #[test]
    fn data_rule_matches_conflict_data() {
        let c = conflict("sys_a", "sys_b", &["Transform", "GlobalTransform"]);
        assert!(AllowRule::Data {
            name: "Transform".into()
        }
        .allows(&c));
        assert!(!AllowRule::Data {
            name: "Velocity".into()
        }
        .allows(&c));
    }

    #[test]
    fn system_rule_matches_either_side() {
        let c = conflict("sys_a", "sys_b", &["Hp"]);
        assert!(AllowRule::System {
            name: "sys_a".into()
        }
        .allows(&c));
        assert!(AllowRule::System {
            name: "::sys_b".into()
        }
        .allows(&c));
        assert!(!AllowRule::System {
            name: "sys_z".into()
        }
        .allows(&c));
    }

    #[test]
    fn world_only_conflicts_are_structurally_flagged() {
        assert!(conflict("a", "b", &[]).is_world_only());
        assert!(!conflict("a", "b", &["Transform"]).is_world_only());
    }
}
