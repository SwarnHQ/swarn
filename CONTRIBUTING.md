# Contributing

Thanks for wanting to help. Swarn is early and moves fast, so please open an issue before starting
anything bigger than a bug fix. Then we can agree on the shape before you spend time on it.

## Before you send a pull request

Run the same checks CI runs:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 -I scripts/license_audit.py
```

The license audit fails if a new dependency brings in a license we haven't reviewed. If you add a
crate and it fails, say so in the pull request rather than editing the reviewed list yourself.

## Ground rules for the code

These come from the [technical PRD](docs/TECHNICAL_PRD.md) and they're worth protecting in review:

- The daemon owns every agent process. If the desktop app ever spawns an agent, that's a bug.
- `swarn-core` does no I/O, so it keeps compiling for iOS without a runtime.
- Clients ignore event kinds they don't know instead of failing.
- Every non-trivial behaviour leaves behind one test that fails if the behaviour breaks.

## Commits and pull requests

Keep each pull request to one change. Write commit messages that say what changed and why, in plain
sentences. By contributing, you agree that your work is licensed under the
[Apache License 2.0](LICENSE).
