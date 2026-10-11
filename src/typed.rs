//! I3: Typed oracle/predicate/policy registry — oracles as Bevy systems.

use bevy::ecs::system::SystemId;
use bevy::ecs::world::World;
use bevy::prelude::*;

use std::collections::HashSet;

use crate::driver::ScenarioResource;
use crate::harness::PlaytestState;
use crate::harness::Violations;
use crate::scenario::Scenario;

/// Result of a typed oracle system. Err(msg) = violation.
pub type OracleResult = Result<(), String>;

/// Registry of typed oracles, predicates, and policies.
#[derive(Resource, Default, Clone)]
pub struct TypedRegistry {
    /// Named oracle systems: () → OracleResult.
    pub oracles: Vec<(String, SystemId<(), OracleResult>)>,
    /// Named goal predicates: () → bool.
    pub predicates: std::collections::HashMap<String, SystemId<(), bool>>,
    /// Named bot policies: () → Option<crate::contract::UserIntent>.
    pub policies:
        std::collections::HashMap<String, SystemId<(), Option<crate::contract::UserIntent>>>,
}

impl TypedRegistry {
    /// FX12 (I3): registered goal-predicate names (load-time
    /// validation of planner trees).
    pub fn predicate_names(&self) -> Vec<String> {
        self.predicates.keys().cloned().collect()
    }
}

/// Extension trait for registering typed oracles/predicates/policies.
pub trait PlaytestAppExt {
    /// Register a named oracle system (evaluated every frame in Oracles set).
    fn add_oracle<M>(
        &mut self,
        name: &str,
        system: impl IntoSystem<(), OracleResult, M> + 'static,
    ) -> &mut Self;
    /// Register a named goal predicate (used by planner for {kind:"predicate"} goals).
    fn add_goal_predicate<M>(
        &mut self,
        name: &str,
        system: impl IntoSystem<(), bool, M> + 'static,
    ) -> &mut Self;
    /// Register a named bot policy (custom bot type).
    fn add_bot_policy<M>(
        &mut self,
        name: &str,
        system: impl IntoSystem<(), Option<crate::contract::UserIntent>, M> + 'static,
    ) -> &mut Self;
}

impl PlaytestAppExt for App {
    fn add_oracle<M>(
        &mut self,
        name: &str,
        system: impl IntoSystem<(), OracleResult, M> + 'static,
    ) -> &mut Self {
        let world = self.world_mut();
        let id = world.register_system(system);
        // 716.30: never panic on a normal app — insert the registry
        // on first use instead of requiring init_resource upfront.
        world.get_resource_or_insert_with::<TypedRegistry>(Default::default);
        world.resource_scope(|_world, mut registry: Mut<TypedRegistry>| {
            registry.oracles.push((name.to_string(), id));
        });
        self
    }

    fn add_goal_predicate<M>(
        &mut self,
        name: &str,
        system: impl IntoSystem<(), bool, M> + 'static,
    ) -> &mut Self {
        let world = self.world_mut();
        let id = world.register_system(system);
        // 716.30: never panic on a normal app — insert the registry
        // on first use instead of requiring init_resource upfront.
        world.get_resource_or_insert_with::<TypedRegistry>(Default::default);
        world.resource_scope(|_world, mut registry: Mut<TypedRegistry>| {
            registry.predicates.insert(name.to_string(), id);
        });
        self
    }

    fn add_bot_policy<M>(
        &mut self,
        name: &str,
        system: impl IntoSystem<(), Option<crate::contract::UserIntent>, M> + 'static,
    ) -> &mut Self {
        let world = self.world_mut();
        let id = world.register_system(system);
        // 716.30: never panic on a normal app — insert the registry
        // on first use instead of requiring init_resource upfront.
        world.get_resource_or_insert_with::<TypedRegistry>(Default::default);
        world.resource_scope(|_world, mut registry: Mut<TypedRegistry>| {
            registry.policies.insert(name.to_string(), id);
        });
        self
    }
}

/// Run all registered typed oracles that are enabled for the current scenario.
/// Exclusive system: needs &mut World to call systems by ID and report violations.
pub(crate) fn run_typed_oracles(world: &mut World) {
    let Some(scenario) = world.get_resource::<crate::driver::ScenarioResource>() else {
        return;
    };
    // 716.30: borrow the oracle list without cloning the full
    // Scenario (input vectors and choice streams are expensive) and
    // without REMOVING TypedRegistry — oracle systems whose params
    // include Res<TypedRegistry> previously failed with a masked
    // "registry error" attributed to the game.
    let oracle_names: Option<Vec<String>> = scenario.0.oracles.clone();
    let to_run: Vec<(String, SystemId<(), OracleResult>)> = {
        let Some(registry) = world.get_resource::<TypedRegistry>() else {
            return;
        };
        match &oracle_names {
            Some(names) => registry
                .oracles
                .iter()
                .filter(|(name, _)| names.contains(name))
                .cloned()
                .collect(),
            None => registry.oracles.clone(),
        }
    };

    let mut violations = world.remove_resource::<Violations>().unwrap_or_default();

    let frame = world.resource::<PlaytestState>().frame;

    // Run each oracle; collect errors into violations. The registry
    // stays in the world so oracle systems can read Res<TypedRegistry>.
    for (name, id) in to_run {
        let result = world.run_system(id);
        match result {
            Ok(Ok(())) => {}
            Ok(Err(msg)) => {
                violations.report_named(&name, "", msg, frame);
            }
            Err(_) => {
                // System registry error — treat as violation.
                violations.report_named(
                    &name,
                    "",
                    format!("oracle '{}' failed to run (registry error)", name),
                    frame,
                );
            }
        }
    }

    world.insert_resource(violations);
}

/// Load-time validation: reject unknown oracle names in scenario.oracles.
pub(crate) fn validate_oracle_names(
    world: &World,
    scenario: &Scenario,
) -> Result<(), crate::scenario::ScenarioError> {
    let Some(names) = &scenario.oracles else {
        return Ok(());
    };

    let registry = world.get_resource::<TypedRegistry>().ok_or_else(|| {
        crate::scenario::ScenarioError::Rejected(
            "scenario specifies oracles but no TypedRegistry is registered".into(),
        )
    })?;

    let known: HashSet<_> = registry.oracles.iter().map(|(n, _)| n.as_str()).collect();
    let unknown: Vec<_> = names
        .iter()
        .filter(|n| !known.contains(n.as_str()))
        .cloned()
        .collect();

    if !unknown.is_empty() {
        return Err(crate::scenario::ScenarioError::Rejected(format!(
            "unknown oracle(s): {:?}; known: {:?}",
            unknown,
            known.into_iter().collect::<Vec<_>>()
        )));
    }

    Ok(())
}

/// Custom bot policy system (I3): runs the named typed policy every
/// frame, paced by input_rate_hz, writing the returned UserIntent.
/// Runs when BotType::Custom; the policy name was validated at load.
pub(crate) fn custom_policy_bot_system(world: &mut World) {
    let Some(scenario) = world.get_resource::<ScenarioResource>() else {
        return;
    };
    let scenario = scenario.0.clone();
    let (frame, tps) = {
        let state = world.resource::<PlaytestState>();
        (state.frame, state.tps)
    };
    let rate = scenario.bot.input_rate_hz.max(1) as u64;
    let fire_every = (tps / rate).max(1);
    if !frame.is_multiple_of(fire_every) {
        return;
    }
    let Some(policy_name) = scenario.bot.policy.clone() else {
        world.resource_mut::<Violations>().report(
            crate::rules::Rule::CustomBotConfig,
            "",
            "custom bot requires `policy` (registered policy name)".to_string(),
            frame,
        );
        return;
    };
    let registry = world.remove_resource::<TypedRegistry>().unwrap_or_default();
    let Some(id) = registry.policies.get(&policy_name).copied() else {
        world.insert_resource(registry);
        world.resource_mut::<Violations>().report(
            crate::rules::Rule::CustomBotConfig,
            "",
            format!("policy '{}' is not registered", policy_name),
            frame,
        );
        return;
    };
    world.insert_resource(registry);
    match world.run_system(id) {
        Ok(Some(intent)) => {
            world.write_message(intent);
            let state = &mut world.resource_mut::<PlaytestState>();
            let mut ctx = std::collections::HashMap::new();
            ctx.insert("policy".to_string(), policy_name.clone());
            state
                .coverage
                .intents_emitted
                .entry(format!("policy:{}", policy_name))
                .and_modify(|c| *c += 1)
                .or_insert(1);
        }
        Ok(None) => {}
        Err(_) => {
            world.resource_mut::<Violations>().report(
                crate::rules::Rule::OracleError,
                "",
                format!("policy '{}' failed to run (registry error)", policy_name),
                frame,
            );
        }
    }
}

/// Load-time validation: a custom bot must name a REGISTERED policy.
pub(crate) fn validate_policy_name(
    world: &World,
    scenario: &Scenario,
) -> Result<(), crate::scenario::ScenarioError> {
    if scenario.bot.bot_type != crate::enums::BotType::Custom {
        return Ok(());
    }
    let Some(name) = &scenario.bot.policy else {
        return Err(crate::scenario::ScenarioError::Rejected(
            "custom bot requires `policy` (registered policy name)".into(),
        ));
    };
    let registry = world.get_resource::<TypedRegistry>();
    let known: Vec<&str> = registry
        .map(|r| r.policies.keys().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    if !known.contains(&name.as_str()) {
        return Err(crate::scenario::ScenarioError::Rejected(format!(
            "unknown bot policy: {:?}; known: {:?}",
            name, known
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<TypedRegistry>();
        app
    }

    #[test]
    fn add_oracle_registers_system() {
        let mut app = build_app();
        app.add_oracle("test_oracle", |_world: &World| Ok(()));
        let registry = app.world().resource::<TypedRegistry>();
        assert_eq!(registry.oracles.len(), 1);
        assert_eq!(registry.oracles[0].0, "test_oracle");
    }

    #[test]
    fn add_goal_predicate_registers_system() {
        let mut app = build_app();
        app.add_goal_predicate("pred", |_world: &World| true);
        let registry = app.world().resource::<TypedRegistry>();
        assert!(registry.predicates.contains_key("pred"));
    }

    #[test]
    fn add_bot_policy_registers_system() {
        let mut app = build_app();
        app.add_bot_policy("policy", |_world: &World| None);
        let registry = app.world().resource::<TypedRegistry>();
        assert!(registry.policies.contains_key("policy"));
    }

    #[test]
    fn validate_unknown_oracle_names_rejected() {
        let mut app = build_app();
        app.add_oracle("known", |_world: &World| Ok(()));
        let scenario: Scenario =
            serde_json::from_str(r#"{"bot":{"type":"replay","inputs":[]},"oracles":["unknown"]}"#)
                .unwrap();
        let err = validate_oracle_names(app.world(), &scenario).unwrap_err();
        match err {
            crate::scenario::ScenarioError::Rejected(msg) => {
                assert!(msg.contains("unknown oracle"));
            }
            other => panic!("expected Rejected, got: {:?}", other),
        }
    }
}

#[cfg(test)]
mod tests_716_30 {
    use super::*;
    use bevy::prelude::App;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Res;

    /// 716.30: add_oracle on an app WITHOUT init_resource::<TypedRegistry>
    /// must not panic — the registry is auto-inserted.
    #[test]
    fn add_oracle_no_panic_without_init() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        // NO init_resource::<TypedRegistry>() here — previously panicked.
        app.add_oracle("o", |_world: &World| Ok(()));
        app.add_goal_predicate("p", |_world: &World| true);
        app.add_bot_policy("b", |_world: &World| None);
        let registry = app.world().resource::<TypedRegistry>();
        assert_eq!(registry.oracles.len(), 1);
        assert!(registry.predicates.contains_key("p"));
        assert!(registry.policies.contains_key("b"));
    }

    /// 716.30: consecutive add_* calls accumulate (the old
    /// clone-modify-reinsert pattern was lossy in principle).
    #[test]
    fn add_calls_accumulate() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_oracle("o1", |_world: &World| Ok(()));
        app.add_oracle("o2", |_world: &World| Ok(()));
        app.add_oracle("o3", |_world: &World| Ok(()));
        assert_eq!(app.world().resource::<TypedRegistry>().oracles.len(), 3);
    }

    /// 716.30: oracles whose params include Res<TypedRegistry> must run
    /// (previously the registry was REMOVED while oracles ran, failing
    /// with a masked "registry error").
    #[test]
    fn oracle_reading_registry_runs() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(Violations::default());
        app.add_oracle(
            "reads_registry",
            |registry: Res<TypedRegistry>| -> OracleResult {
                if registry.oracles.is_empty() {
                    Err("empty registry".to_string())
                } else {
                    Ok(())
                }
            },
        );
        // ScenarioResource + PlaytestState so run_typed_oracles runs.
        let scenario: Scenario =
            serde_json::from_str(r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1}"#)
                .unwrap();
        use std::sync::Arc;
        app.insert_resource(ScenarioResource(Arc::new(scenario)));
        app.insert_resource(crate::state::PlaytestState::new(60, 1));

        run_typed_oracles(app.world_mut());
        let v = app.world().resource::<crate::state::Violations>();
        assert!(
            v.entries.is_empty(),
            "oracle with Res<TypedRegistry> param must run cleanly, got: {:?}",
            v.entries
        );
    }
}
