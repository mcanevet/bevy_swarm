# cargo-bevy-playtest

CLI scaffolding for [bevy_playtest](https://crates.io/crates/bevy_playtest) playtests.

Installed as `cargo-bevy-playtest` and invoked through cargo as
`cargo bevy-playtest <SUBCOMMAND>`.

## Install

```sh
cargo install cargo-bevy-playtest
```

## Usage

```sh
cargo bevy-playtest init       # detect the game's root plugin, generate tests/playtest.rs
cargo bevy-playtest run        # run the generated playtest test (env overrides)
cargo bevy-playtest validate <file>   # check a scenario JSON without running the game
cargo bevy-playtest reproduce  # re-run a failing seed from the last report
cargo bevy-playtest report     # pretty-print the last autotest report
```

### Optional shorter alias

Add to your project's `.cargo/config.toml`:

```toml
[alias]
swarm = "bevy-playtest"
```

then use `cargo swarm <SUBCOMMAND>`.

## Environment overrides

- `BEVY_PLAYTEST_SEEDS` / `BEVY_PLAYTEST_DURATION` / `BEVY_PLAYTEST_THREADS`
- `BEVY_PLAYTEST_SEED=<n>` pins one exact seed (reproduction)
- Legacy `BEVY_SWARM_*` spellings are honored when the new name isn't set.

## License

Licensed under either of MIT or Apache-2.0, at your option.
