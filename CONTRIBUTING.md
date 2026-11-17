# Contributing

Thanks for helping. Drainlog has one job: to explain overnight battery drain honestly.

## Build and test

```bash
scripts/bundle.sh            # builds build/Drainlog.app
(cd core && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check)
(cd app && swift build -Xswiftc -warnings-as-errors && swift test)
```

## Pull requests

- Keep each PR to one change. Use [Conventional Commits](https://www.conventionalcommits.org) for commits and the PR title, for example `fix(core): ...`.
- Add or update a test for any change to parsing, the ledger or causes. Use the fixtures in `core/tests/fixtures/`. Mark synthetic fixtures as synthetic.
- Never show an estimated number as measured. Keep the Unexplained line visible.
- Remove usernames, hostnames and machine IDs from any log you attach or commit. `judge/capture.sh` does this for you.
- CI must pass.
