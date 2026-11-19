# Contributing

## The basics

fetch is a small Rust crate. The only dependency is `libc`. Keep it that way :)

Before submitting a PR, run `make test` (or `cargo test` and `python3 tests/posix.py` after a
`cargo build --release`) and make sure everything passes. `cargo clippy --all-targets` should
stay clean.

## What makes a good PR

- **Small and focused.** One fix or one feature per PR. If your diff is 500+ lines, it's probably too big. Break it up or open an issue first to discuss scope.
- **No new dependencies.** Please don't add crates and don't shell out to external tools beyond what's already used.
- **No unnecessary refactors.** If you're not fixing a bug or adding a feature, don't move code around. "Cleaner" is subjective and adds review burden for no user-facing benefit.
- **Test your changes.** `cargo test` runs the unit tests (next to the code they cover) and `tests/posix.py` runs the end-to-end regression tests against the built binary (set `FETCH_BIN` to test a different build). If you're adding a new feature, add a test for it.

## What will get your PR rejected

- **Scope creep.** A PR that adds a small feature buried in 200 lines of unrelated cleanup won't get merged. Split code changes up if they are unrelated.
- **Untested drive-by "fixes."** If you're changing behavior, show that it was broken before and works now.
- **Fake security patches.** Adding needless validation around safe code to pad a PR is obvious and will be closed.

## Security

fetch has a strict policy on command execution:

- **No command execution from config files.** Config values are static strings only. Features that let users run arbitrary commands from config will not be merged.
- **Never go through a shell.** External tools are run with `std::process::Command` and an argument list, so data read from the system can't be interpreted as shell syntax. Validate any value that becomes a path or program name (see `cmd_version`).
- **Use `-c none` for any fastfetch calls** to ignore user configs.

## AI-generated code

If AI was involved in writing your PR, please say so! PRs that are clearly AI-generated and don't disclose it **will** be closed (regardless of code quality).

Don't use AI to generate issues, discussion posts, or governance proposals. There is absolutely **zero** tolerance for those things being AI generated.

## Style

- Stable Rust, 2021 edition
- Match the existing code style. Look at the surrounding module before writing
- No over-commenting. If the code needs a paragraph to explain, simplify the code
- Keep error messages short and useful: `eprintln!("fetch: ...")` format

## Layout

- `src/main.rs`: CLI, startup, the animation loop
- `src/config.rs`: `~/.config/fetch/config`
- `src/logo.rs`: logo loading (file, fastfetch, built-in), distro detection, relief weights
- `src/render.rs`: point cloud, layout, rasterizing, frame composition
- `src/shading.rs`: shading ramps and sub-cell glyphs
- `src/info.rs`: the info block (lines, live refresh, box)
- `src/minimal.rs`: `--minimal`, the one-shot static print
- `src/term.rs`: raw mode, signals, mouse/keyboard input
- `src/gather/`: the system-info gatherers

## Questions?

Open an issue!
