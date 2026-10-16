# Swarn

Run coding agents on your Mac and stay on top of them from your phone.

Swarn starts Claude Code and Codex sessions in their own git worktrees, shows what each one is doing,
and sends an end-to-end encrypted push to your iPhone when one of them needs you. You approve, deny
or reply from the notification, read the diff on the desktop, and open the pull request with `gh`.

> **Status: pre-alpha.** This is week 4 of an eight-week build toward v0.1. The desktop app runs
> Claude Code and Codex in git worktrees, shows their transcripts and diffs, asks before they write
> or run anything, notifies you when one needs you, and opens pull requests with `gh`. The iPhone
> app comes next. Nothing here is ready for daily use yet.

## How it fits together

| Piece | What it does |
| --- | --- |
| `swarnd` (daemon) | Owns the agent processes, worktrees, event log and pairings. Keeps running when the app quits. |
| Swarn.app (desktop) | GPUI app that renders what the daemon reports. It holds no agent state of its own. |
| Swarn for iOS | Pairs by QR code, receives encrypted pushes, signs approvals with the Secure Enclave. |
| Push gateway | Stateless Cloudflare Worker that forwards sealed blobs to APNs. It never sees plaintext. |

The full design is in [docs/TECHNICAL_PRD.md](docs/TECHNICAL_PRD.md), and the open questions about
it are in [docs/PRD_REVIEW.md](docs/PRD_REVIEW.md).

## Building

You need macOS on Apple silicon, Xcode (GPUI compiles Metal shaders at build time) and rustup. The
pinned Rust version installs itself from `rust-toolchain.toml` the first time you run cargo.

Start the daemon in one terminal:

```bash
cargo run -p swarn-daemon
```

Then start the desktop app in another:

```bash
cargo run -p swarn-desktop
```

Add a repository and it shows up in the window within two seconds:

```bash
cargo run -p swarn-daemon -- call repo.add '{"path":"/path/to/a/git/repo"}'
```

From there you can run Claude Code in its own worktree and approve its tool calls from a terminal.
`swarnd` here is `target/debug/swarnd`, and the ids come from each command's output:

```bash
swarnd call workspace.create '{"repo_id":"<repo id>","name":"first-task"}'
```

```bash
swarnd watch
```

```bash
swarnd call session.start '{"workspace_id":"<workspace id>","prompt":"Add a README section"}'
```

The desktop app shows the transcript and an approval card whenever the agent asks for permission.
`swarnd watch` does the same in a terminal and prints the exact `approval.resolve` command. Sessions use your own `claude` and your own login, so sign in with
`claude /login` first.

Until the iPhone app exists, `swarn-fake-phone` stands in for it. It pairs with the running daemon
over the real sealed protocol, prints every event, and with `--approve` answers permission requests
with signed approvals:

```bash
cargo run -p swarn-bench --bin swarn-fake-phone -- --approve
```

The daemon keeps its socket and database in `~/Library/Application Support/swarn`. Set `SWARN_HOME`
on both processes to point them somewhere else, which is handy for experiments.

## Repository layout

```
crates/
  swarn-core/      shared types and session schema, no I/O
  swarn-daemon/    the swarnd binary: SQLite, local JSON-RPC API
  swarn-desktop/   the GPUI app, binary `swarn`
docs/              design documents and the license audit
scripts/           maintenance scripts
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Please report security issues privately as described in
[SECURITY.md](SECURITY.md), not in public issues.

## License

Swarn is licensed under the [Apache License 2.0](LICENSE).

The desktop look is adapted from [Plexo](https://github.com/anmolkapil/plexo) (MIT). Manrope and
Roboto Mono are bundled under the SIL Open Font License 1.1; see
[crates/swarn-desktop/assets/fonts](crates/swarn-desktop/assets/fonts/README.md).
