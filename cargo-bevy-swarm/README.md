# cargo-bevy-swarm

CLI scaffolding for [bevy_swarm](https://crates.io/crates/bevy_swarm) playtests.

Installed as `cargo-bevy-swarm` and invoked through cargo as
`cargo bevy-swarm <SUBCOMMAND>`.

## Install

```sh
cargo install cargo-bevy-swarm
```

## Usage

```sh
cargo bevy-swarm init       # detect the game's root plugin, generate tests/swarm.rs
cargo bevy-swarm run        # run the generated swarm test (env overrides)
cargo bevy-swarm validate <file>   # check a scenario JSON without running the game
cargo bevy-swarm reproduce  # re-run a failing seed from the last report
cargo bevy-swarm report     # pretty-print the last autotest report
```

### Optional shorter alias

Add to your project's `.cargo/config.toml`:

```toml
[alias]
swarm = "bevy-swarm"
```

then use `cargo swarm <SUBCOMMAND>`.

## License

Licensed under either of MIT or Apache-2.0, at your option.
