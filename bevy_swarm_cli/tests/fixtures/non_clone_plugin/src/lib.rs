use bevy::prelude::*;

#[derive(Default)]
pub struct GamePlugin {
    state: std::rc::Rc<u8>,
}

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, || {});
    }
}
