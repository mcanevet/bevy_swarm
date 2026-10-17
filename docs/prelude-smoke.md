```rust,no_run
// The SUPPORTED surface of bevy_playtest 0.1: the quick-start pattern
// compiles with ONLY the prelude.
use bevy::prelude::*;
use bevy_playtest::prelude::*;

#[derive(Component)]
struct Player;

#[derive(Resource, Default)]
struct Score(i32);

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TestConventionsPlugin, PlaytestPlugin));
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Choice(3),
        SurfaceVariant::Wait,
    ]));
    app.insert_resource(Score(0));
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Player, Transform::default()));
    });

    let scenario = serde_json::from_str::<Scenario>(
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":0.1,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let report = run_scenario(&mut app, &scenario).expect("run scenario");
    println!("status: {:?}", report.status);
}
```
