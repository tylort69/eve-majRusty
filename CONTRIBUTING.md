# Contributing

Thanks for helping out. Bug reports, ideas and pull requests are all welcome.

## Before you start

- For anything bigger than a small fix, open an issue first so we can agree on the approach.
- Keep pull requests focused on one change.
- Never commit real character names, character IDs, system names, chat lines or screenshots from your own game.
  Tests and docs use made-up pilots ("Pilot Alpha") and log lines.

## Building and testing

See the README for the Linux dependencies. Then:

```sh
cargo t        # unit tests (Linux host)
cargo linux    # release build of eve-maj-preview and eve-maj-config
cargo build    # Windows cross-build (needs MinGW)
cargo clippy --target x86_64-unknown-linux-gnu
```

CI runs the tests and both builds on every pull request.

## Code layout

- Platform-independent logic belongs in `crates/core`, with unit tests.
- The Linux front end is `crates/preview/src/linux/` and `crates/config/src/linux/`.
- The settings page (`crates/config/ui/`) comes from the Windows build and should stay usable by a future
  `config.exe` port: keep Linux-only logic in the Rust backend, and add new UI strings to every file in `ui/lang/`.
- Profiles must stay compatible with the original EVE-Maj Preview's JSON format.

## License

By contributing, you agree that your contribution is licensed under the GPL-3.0, like the rest of the project.
