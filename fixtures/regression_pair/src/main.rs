//! Fixture: regression_pair game

use bevy::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(fixture_regression_pair::RegressionPairGamePlugin);
    app.run();
}
