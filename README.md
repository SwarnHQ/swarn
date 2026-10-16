# Swarn

Run coding agents on your Mac and stay on top of them from your phone.

Swarn starts Claude Code and Codex sessions in their own git worktrees, shows what each one is doing,
and sends an end-to-end encrypted push to your iPhone when one of them needs you. You approve, deny
or reply from the notification, read the diff on the desktop, and open the pull request with `gh`.

> **Status: pre-alpha.** The desktop side of v0.1 is in place. The app runs Claude Code and Codex
> in git worktrees, shows their transcripts and diffs, and asks before they write or run anything.
> It notifies you when an agent needs you and opens pull requests with `gh`. It also pairs with a
> phone over an end-to-end encrypted channel. The iPhone app and the push gateway aren't built
> yet, and there's no signed release. Expect rough edges.

## Installing

There's no signed release yet. Once one is published, this installs it into `/Applications`:

```bash
curl -fsSL https://raw.githubusercontent.com/SwarnHQ/swarn/main/scripts/install.sh | sh
```

Until then, build the disk image yourself. It's unsigned, so it opens on the Mac that built it:

```bash
scripts/dmg.sh
```

Open `target/release/Swarn-<version>.dmg` and drag Swarn to Applications. On first launch, Swarn
asks to run its daemon in the background. It shows up in System Settings under Login Items, where
you can turn it off again.

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
  swarn-core/      shared types, session schema and the crypto envelope
  swarn-daemon/    the swarnd binary: agents, worktrees, SQLite, local API, device listener
  swarn-desktop/   the GPUI app, binary `swarn`
  swarn-bench/     benchmark harness, a fake agent and a fake phone
docs/              design documents, threat model, license audit
scripts/           bundling, disk image, install and license audit
```

## Releasing

Bump the version in `crates/swarn-desktop/Cargo.toml`, then push a matching tag such as `v0.1.0`.
The release workflow builds the disk image and attaches it to a draft release for someone to
publish. It signs and notarizes the build when the Developer ID secrets listed in
[.github/workflows/release.yml](.github/workflows/release.yml) are set. Without them the build is
unsigned.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Please report security issues privately as described in
[SECURITY.md](SECURITY.md), not in public issues. [docs/threat-model.md](docs/threat-model.md) says what
Swarn defends against.

## License

Swarn is licensed under the [Apache License 2.0](LICENSE).

The desktop look is adapted from [Plexo](https://github.com/anmolkapil/plexo) (MIT). Manrope and
Roboto Mono are bundled under the SIL Open Font License 1.1; see
[crates/swarn-desktop/assets/fonts](crates/swarn-desktop/assets/fonts/README.md).
