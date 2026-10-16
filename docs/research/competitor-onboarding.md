# Competitor onboarding and agent connection

Research date: 16 Oct 2026. Scope: what each product makes a user set up on first run, and how it connects to and drives coding agents. This is input for Swarn's first-run flow, which is not designed yet. PRD references point to `docs/TECHNICAL_PRD.md`.

Method: official docs, sites, changelogs, App Store listings, and each product's own GitHub repo (README, source, issues, releases). Nothing was installed, signed into, or run. Claims carry a link. "Not documented" means I looked and did not find it. "Unverified" marks a secondary source. These products ship weekly, so version numbers and dates matter more than usual.

## 1. Summary

1. Swarn's core loop already exists in first-party tools. `claude remote-control --spawn worktree` runs up to 32 worktree sessions, pairs the Claude iOS app by QR, and pushes permission prompts to the phone ([Remote Control](https://code.claude.com/docs/en/remote-control)). The ChatGPT desktop app does the same for Codex ([mobile](https://learn.chatgpt.com/docs/mobile.md)). Both relay through the vendor, and Anthropic stores the transcript on its servers. Neither documents end-to-end encryption.
2. Direct third-party rivals with phone approvals: Happy (relay, E2E sync, plaintext Expo pushes), Nimbalyst (formerly Crystal; E2E sync, plaintext APNs titles), and cmux (E2E pushes since 0.65.0, 5 Oct 2026). Conductor's iOS app (Oct 2026) works only with paid cloud workspaces. None of them offers a phone-to-Mac path that skips their relay.
3. The adapters the PRD names have moved. `zed-industries/codex-acp` is archived. Its replacement, `@agentclientprotocol/codex-acp`, is a Node package that wraps `codex app-server`. The Claude adapter is now `@agentclientprotocol/claude-agent-acp`. It embeds the Claude Agent SDK and a pinned CLI, so it does not run the user's installed `claude`.
4. The mature tools drive agents through vendor protocols: the Claude Agent SDK or `claude -p --output-format stream-json`, and `codex app-server`. Only Emdash and Nimbalyst use ACP for Claude or Codex. Both tools that scraped a terminal (Omnara v1, Claude Squad) broke on CLI updates.
5. Agent auth: everyone reuses the user's existing CLI login and offers an API key fallback. Anthropic forbids third-party apps from offering Claude.ai login or touching subscription tokens. Users signing in to the unmodified `claude` binary themselves is allowed ([legal](https://code.claude.com/docs/en/legal-and-compliance)). Conductor handles this with an embedded terminal that runs `claude /login`.
6. Conductor bundles pinned Claude Code, Codex, OpenCode and even `gh`, and lets users switch to the system binary. Vibe Kanban pinned an npm version that later vanished and broke first runs.
7. `gh` is the common GitHub path (Conductor, Sculptor, Superset, Vibe Kanban, Nimbalyst, Claude Squad). Conductor later added `GH_TOKEN`, PATs and a GitHub App because `gh` detection kept failing. It also checks for an expired Xcode license, which silently breaks `gh`.
8. Background process: Conductor has none, and local sessions die when the app quits. Superset, Happy and Omnara run a helper that survives the UI. Only Omnara's `omnarad` registers a per-user launchd service. Every tool says the Mac must stay awake, and most offer a keep-awake toggle.
9. Worktree conventions are converging: worktrees outside the repo, setup/run/archive scripts, `.env*` copying via `.worktreeinclude` (shared with Claude Code), and a 10-port range per workspace (Conductor, Emdash).
10. Most competitors default to skipping permissions (Superset, Vibe Kanban, Sculptor, Crystal). Conductor kept tool approvals off until an opt-in in April 2026. Happy Desktop defaults to an AI reviewer. A human-approval default with a clear policy choice is open ground.
11. Agents spawned from a daemon or as child processes can fail to authenticate when the macOS Keychain or a login-shell `PATH` is missing. That bears directly on PRD §1.1 (launchd user agent).

## 2. Comparison table

| Product | Install | Agent connection | Agent auth | Mobile and approvals | Background helper |
| --- | --- | --- | --- | --- | --- |
| Conductor | DMG, macOS only ([install](https://www.conductor.build/docs/installation)) | Bundled Claude Code via Claude Agent SDK; bundled Codex (own SDK path); Cursor API; OpenCode | Reuses CLI login; API key mode in settings; Sign in with ChatGPT (0.89.0) | Desktop approvals (opt-in) and macOS notifications; iOS app for cloud workspaces only (Pro) | None; local sessions end when the app closes ([pricing FAQ](https://www.conductor.build/pricing)) |
| Happy | `npm i -g happy`; Desktop DMG or brew cask; App Store | CLI: Claude Agent SDK `query()` plus local `claude`; `codex app-server`; generic ACP. Desktop: own harness calling vendor APIs | Reuses CLI logins; Desktop reads Claude Keychain token and `~/.codex/auth.json` | QR pairing, E2E sync via relay, Expo push with plaintext titles, approvals from phone | CLI auto-starts a detached daemon; optional sudo LaunchDaemon; Desktop starts Happy Agent |
| Omnara | Now `npx omnara` plus `omnarad` | v1 PTY scraping, v2 Agent SDK; now its own agent loop | Now API keys in Omnara secrets | Old mobile app gone; approvals via dashboard, Slack, Discord | `omnarad` installs a launchd user service |
| Nimbalyst (was Crystal) | DMG, Windows, Linux; iOS App Store | Claude Agent SDK; `codex app-server`; ACP for Copilot and Grok | Bundled runtime; login via Terminal or API key | QR pairing after account sign-in; E2E sync; APNs push with plaintext titles; phone approvals | Desktop must stay awake; early headless host |
| Claude Squad | brew or curl script | Interactive CLI in tmux, screen-scraped | CLI login | None | tmux sessions survive; autoyes daemon |
| Vibe Kanban (sunsetting) | `npx vibe-kanban` or Tauri app | `npx` pinned Claude Code stream-json; `codex app-server`; ACP for Gemini, Qwen, Copilot | CLI login; optional VK account | Web remote via relay (status unclear); no native app | Tauri app hides instead of quitting |
| Sculptor | DMG, AppImage | `claude` stream-json with skip-permissions | CLI login | None; no tool approvals | Not documented |
| Superset | DMG; CLI via brew; iOS App Store | PTY terminals plus installed hooks; Agent SDK for chat pane | CLI login or API key | Relay (no E2E found); phone push "coming"; Claude launched with skip-permissions by default | `pty-daemon` survives quit; host stops on quit |
| cmux | DMG or brew cask; iOS TestFlight | Plain PTY terminal plus agent hooks | CLI login | E2E push and phone approvals via `PermissionRequest` hook (120 s hold) | None; terminals live in the app |
| Emdash | DMG or brew cask | ACP adapters (`claude-agent-acp`, `codex-acp`) plus PTY | CLI login | No mobile | tmux-backed sessions |
| Claude Code (first party) | CLI, Desktop app | Native | claude.ai login or API key | Remote Control: QR, push on permission prompts, relay via Anthropic | `claude remote-control` server mode in a terminal |
| Codex / ChatGPT app | CLI, ChatGPT desktop app | Native; app-server for clients | ChatGPT login or API key | QR pairing, phone approvals, "secure relay layer" | Desktop app must stay open and awake |
| Zed | Zed editor | ACP adapters from the ACP registry, pinned npx versions | Adapter auth methods; `/login` in a Claude thread | No mobile | None |

## 3. Products

### 3.1 Conductor (Melty Labs)

Conductor is the most complete Mac product in this space and the closest match to Swarn's desktop half. Current version 0.91.0 (15 Oct 2026) ([changelog](https://www.conductor.build/changelog)). Closed source. The docs moved from docs.conductor.build to [conductor.build/docs](https://www.conductor.build/docs/).

Distribution and pricing

- DMG for macOS. No Windows or Linux ([install](https://www.conductor.build/docs/installation)).
- Free tier: local workspaces, bring your own subscriptions and keys. Pro ($50/month) adds Conductor Cloud, the API and the mobile app. Teams is $60/user/month ([pricing](https://www.conductor.build/pricing)).
- iOS app "Conductor.build", iPhone only, iOS 26+, version 1.0 on 14 Sep 2026, 1.0.8 by 15 Oct ([App Store](https://apps.apple.com/us/app/conductor-build/id6791228564)). The 0.90.0 release note says it "Works with cloud workspaces" ([0.90.0](https://www.conductor.build/changelog/0.90.0-conductor-for-ios)).

First run

1. Drag to Applications and open ([install](https://www.conductor.build/docs/installation)).
2. Setup checks. On launch Conductor checks for "GitHub authentication in your terminal environment" (`gh auth status`), Claude Code login (`claude /login`), Codex login (`codex login`), and a Cursor API key. "You need GitHub and at least one agent provider to use Conductor." If anything is missing, "Conductor walks you through setup" ([install](https://www.conductor.build/docs/installation)).
3. Add a repository: Open project (local), Open GitHub project, or Quick start (new repo) ([first workspace](https://www.conductor.build/docs/first-workspace)). An "Add your first project" sidebar button appears during onboarding (0.67.0, [page 4](https://www.conductor.build/changelog/page/4)).
4. A guided sample project ("Starter project") was added in 0.52.0 and renamed in 0.67.0 ([page 5](https://www.conductor.build/changelog/page/5), [page 4](https://www.conductor.build/changelog/page/4)).
5. Configure the project. The docs suggest pasting a prompt that asks the agent to inspect the repo and write `.conductor/settings.local.toml` with setup, run and archive scripts ([configure your project](https://www.conductor.build/docs/configure-your-project)).

Onboarding has been redesigned repeatedly: 0.1.0 (Jul 2025), 0.11.0, 0.15.2, 0.79.0 ([page 21](https://www.conductor.build/changelog/page/21), [page 18](https://www.conductor.build/changelog/page/18), [page 16](https://www.conductor.build/changelog/page/16), [page 3](https://www.conductor.build/changelog/page/3)). Users can step back between onboarding steps since 0.49.0 ([page 6](https://www.conductor.build/changelog/page/6)).

Conductor accounts: the install docs list no account step. The changelog shows local use is tied to an account anyway. 0.50.0 "uses your last signed-in account to load when offline" ([page 6](https://www.conductor.build/changelog/page/6)), and 0.82.0 fixed "local workspaces being hidden when they were created under another Conductor account" ([page 2](https://www.conductor.build/changelog/page/2)). Browser sign-in uses PKCE since 0.80.0 ([page 2](https://www.conductor.build/changelog/page/2)).

macOS permissions: Conductor does not ask for TCC permissions itself. But when an agent reads Downloads, Desktop or Reminders, macOS names Conductor in the prompt "because Conductor is the app launching the process" ([security](https://www.conductor.build/docs/reference/security-and-permissions), [FAQ](https://www.conductor.build/docs/faq)). Since 0.74.0 it recommends Full Disk Access to stop repeated prompts ([page 3](https://www.conductor.build/changelog/page/3)). 0.90.0 fixed macOS asking again even after Full Disk Access ([0.90.0](https://www.conductor.build/changelog/0.90.0-conductor-for-ios)).

Agent connection

- "Claude Code, Codex, and OpenCode come bundled in Conductor. You do not need to install them separately." Cursor runs through the Cursor API ([harnesses](https://www.conductor.build/docs/reference/harnesses)).
- Bundled binaries live in `~/Library/Application Support/com.conductor.app/bin`: "Do not update or modify them, or they might be incompatible with Conductor" ([FAQ](https://www.conductor.build/docs/faq)). Each release bumps them, for example Claude Code 2.1.300 and Codex 0.162.1 in 0.91.0 ([changelog](https://www.conductor.build/changelog)).
- Claude Code is driven through the Claude Agent SDK. The changelog tracks SDK upgrades, for example "Anthropic Agent SDK to 2.1.50" for memory leaks in 0.36.3 and 0.2.64 in 0.38.0 ([0.36.3](https://www.conductor.build/changelog/0.36.3-submit-a-prompt-sdk-memory-leaks), [page 7](https://www.conductor.build/changelog/page/7)).
- Codex: plan mode arrived behind "Settings > Experimental > Use alternative Codex SDK" in 0.41.0 ([0.41.0](https://www.conductor.build/changelog/0.41.0-tool-approval-codex-plan-mode-beta)). Which Codex protocol it uses today is not documented.
- No ACP is mentioned anywhere in the docs.
- Escape hatch: Settings > Storage > "Use system Claude Code (from PATH)" or a manual path, for users whose setup depends on PATH tools or custom config ([troubleshooting](https://www.conductor.build/docs/troubleshooting/issues)). Managed settings can force `claude_code_executable_path` ([managed settings](https://www.conductor.build/docs/reference/settings/managed)).
- Shell environment: Conductor launches an interactive login shell in each workspace to capture env vars, then runs scripts with zsh. It gives the shell config 5 seconds and aborts after that ([shells](https://www.conductor.build/docs/reference/shells)). Fish support and zsh fallback came in 0.52.0 ([0.52.0](https://www.conductor.build/changelog/0.52.0-sounds-colors)).
- Conductor injects its own system prompts, visible and editable in repository preferences ([FAQ](https://www.conductor.build/docs/faq)).

Agent auth

- "By default, Conductor uses the auth tokens already saved on your machine", whether that is a Claude API key or a Pro/Max login. Users can switch to API key mode in Settings > Harnesses ([FAQ](https://www.conductor.build/docs/faq), [providers](https://www.conductor.build/docs/guides/providers)).
- Warning in the docs: if `ANTHROPIC_API_KEY` is set in Conductor or the shell that launches it, Claude Code may bill the API instead of the subscription ([Claude Code harness](https://www.conductor.build/docs/reference/harnesses/claude-code)).
- Claude login happens in an embedded terminal running `claude /login`. Since 0.51.0 that terminal closes on success and the status flips to "Logged in" ([page 6](https://www.conductor.build/changelog/page/6)). Several releases fixed this path: 0.25.9 (onboarding broke for users not signed in to Claude Code or Codex), 0.27.2 (Claude auth hung during onboarding), 0.55.0, 0.57.0, 0.62.0 ([page 13](https://www.conductor.build/changelog/page/13), [page 12](https://www.conductor.build/changelog/page/12), [page 5](https://www.conductor.build/changelog/page/5)). 0.25.7 suggested `claude setup-token` for OAuth timeouts ([page 13](https://www.conductor.build/changelog/page/13)).
- Codex: CLI login or `CODEX_API_KEY`. "Sign in with ChatGPT" partnership in 0.89.0 (29 Sep 2026) ([0.89.0](https://www.conductor.build/changelog/0.89.0-sign-in-with-chatgpt)). 0.76.0 fixed Codex reporting "CLI not authenticated" when credentials were in the macOS Keychain ([page 3](https://www.conductor.build/changelog/page/3)).
- Conductor's own stored keys moved to Keychain-backed storage in 0.69.0 ([page 4](https://www.conductor.build/changelog/page/4)).

Repos and workspaces

- Worktree per workspace under `~/conductor/workspaces/<repo>/<workspace>`. Older versions nested them in `<repo>/.conductor`, which confused build tools ([git worktrees](https://www.conductor.build/docs/concepts/git-worktrees), [troubleshooting](https://www.conductor.build/docs/troubleshooting/issues)).
- Workspace directories are named after cities (295 of them), separate from the branch name ([cities](https://www.conductor.build/docs/reference/cities)).
- New branch from the configured base, after fetching `origin`. On the first chat the agent is told to rename the branch ([isolated workspaces](https://www.conductor.build/docs/concepts/workspaces-and-branches)). 0.89.0 made descriptive branch names automatic ([0.89.0](https://www.conductor.build/changelog/0.89.0-sign-in-with-chatgpt)).
- Scripts: `setup` (after creation), `run` (Run button, several allowed), `archive` (before archive), `run_mode`. Configured in-app or in `.conductor/settings.toml` (shared) and `.conductor/settings.local.toml` (personal), layered over `~/.conductor/settings.toml` ([scripts](https://www.conductor.build/docs/reference/scripts), [configure](https://www.conductor.build/docs/configure-your-project)).
- Ports: "Conductor allocates ten ports to each workspace: `CONDUCTOR_PORT` through `CONDUCTOR_PORT+9`" ([scripts](https://www.conductor.build/docs/reference/scripts)). Other env vars: `CONDUCTOR_WORKSPACE_PATH`, `CONDUCTOR_ROOT_PATH`, `CONDUCTOR_DEFAULT_BRANCH` ([env vars](https://www.conductor.build/docs/reference/environment-variables)).
- Files to copy: gitignored files matching a pattern list are copied into new workspaces. Source order is `.worktreeinclude` at the repo root, then repo settings, then the default `.env*` ([files to copy](https://www.conductor.build/docs/reference/files-to-copy)). The docs note `.worktreeinclude` uses the same syntax Claude Code uses for its worktrees.
- Since 0.57.0, sending a message no longer waits for the setup script ([0.57.0](https://www.conductor.build/changelog/0.57.0-sync-agent-configs)). Since 0.45.0, a failed setup script offers "ask the AI to fix it" ([page 6](https://www.conductor.build/changelog/page/6)).
- Archive and restore via a History pane. Settings `git.delete_branch_on_archive` and `git.archive_on_merge` ([troubleshooting](https://www.conductor.build/docs/troubleshooting/issues), [settings reference](https://www.conductor.build/docs/reference/settings/reference)).

GitHub

- Originally `gh` from the terminal environment ([first workspace](https://www.conductor.build/docs/first-workspace)).
- 0.25.5 (Dec 2025) added a `GH_TOKEN` paste "if Conductor has had trouble detecting your gh status" ([page 13](https://www.conductor.build/changelog/page/13)). 0.25.12 started bundling the GitHub CLI: "If you've had issues authenticating with GitHub in the past, this should help!" ([page 12](https://www.conductor.build/changelog/page/12)).
- 0.36.7 checks on startup whether the Xcode license is out of date "to avoid gh commands silently failing" ([page 8](https://www.conductor.build/changelog/page/8)).
- 0.51.0 pre-fills scopes on GitHub's new-token page. 0.65.0 "prefer[s] the GitHub app path and preserve[s] reusable personal access tokens" ([page 6](https://www.conductor.build/changelog/page/6), [page 4](https://www.conductor.build/changelog/page/4)). Exact scopes are not documented.
- PR creation: Create PR sends the diff to the agent to draft the description; the merge happens in-app ([review and merge](https://www.conductor.build/docs/guides/review-and-merge)).

Approvals and notifications

- Agents run unsandboxed with the user's permissions ([FAQ](https://www.conductor.build/docs/faq)).
- Tool approval was an enterprise setting you had to email for (0.41.0, Mar 2026). It became a user toggle in Settings > Account in 0.45.0, which also added "a macOS notification when an agent requests tool permission" ([0.41.0](https://www.conductor.build/changelog/0.41.0-tool-approval-codex-plan-mode-beta), [page 6](https://www.conductor.build/changelog/page/6)).
- 0.70.0 added an experimental "Auto permission mode" that auto-approves safe actions ([page 4](https://www.conductor.build/changelog/page/4)). Settings `tool_approvals_enabled` and `models.codex.tool_approval_policy` (`default` or `always_approve`) are user-only ([settings reference](https://www.conductor.build/docs/reference/settings/reference)).
- Approval bugs in the changelog: approvals skipped (0.54.0), subagents stuck waiting for approval (0.46.0), Codex bypassing approval settings (0.82.0), "local command approval timeouts being reported as user denials" (0.83.0) ([page 5](https://www.conductor.build/changelog/page/5), [page 6](https://www.conductor.build/changelog/page/6), [page 2](https://www.conductor.build/changelog/page/2)).
- Completion sounds and macOS notifications; notifications when Claude has questions or awaits plan review (0.28.7) ([page 11](https://www.conductor.build/changelog/page/11)).

Mobile and remote

- The iOS app covers cloud workspaces: chat with agents, review, merge PRs ([0.90.0](https://www.conductor.build/changelog/0.90.0-conductor-for-ios)). Login is the Conductor account, so there is no device pairing. A local Mac workspace cannot be reached from the phone (not documented as supported).
- An App Store review asks for a notification "when a job is done" ([App Store](https://apps.apple.com/us/app/conductor-build/id6791228564)), so push for agent events looked missing as of Oct 2026.
- Cloud workspaces are Vercel sandboxes that keep running after the Mac closes ([cloud](https://www.conductor.build/docs/cloud), [pricing FAQ](https://www.conductor.build/pricing)).

Background process

- None for local work: "When your machine shuts off or you close the app, sessions terminate" ([pricing FAQ](https://www.conductor.build/pricing)). Since 0.25.4 it asks for confirmation before quitting with a running agent ([page 13](https://www.conductor.build/changelog/page/13)).
- "Caffeinate while agents are running" setting since 0.50.0 ([page 6](https://www.conductor.build/changelog/page/6)).
- Updates can wait until agents are idle (0.39.0, 0.63.0) ([page 7](https://www.conductor.build/changelog/page/7), [0.63.0](https://www.conductor.build/changelog/0.63.0-cursor-support-dispatcher)).

What this teaches

Most onboarding bug fixes are about agent auth detection and GitHub detection, not about worktrees. The fixes followed one pattern: embed the login terminal, bundle the tool, accept a token as fallback, and check for silent killers like the Xcode license.

### 3.2 Happy (slopus)

The closest open-source match to Swarn's phone story. MIT ([repo](https://github.com/slopus/happy)).

Two products now:

- Happy CLI (`happy` on npm, Node 20+). It wraps Claude Code and Codex and talks to a phone app through Happy's relay. Officially in maintenance mode: "it gets critical fixes, and new features ship in Happy Desktop" ([terminal guide source](https://github.com/slopus/slopus.github.io)). Latest 1.2.5 on 22 Sep 2026 ([releases](https://github.com/slopus/happy/releases)).
- Happy Desktop plus Happy Agent (since July 2026). An Electron app ([happy-desktop](https://github.com/slopus/happy-desktop)) plus a headless daemon with its own agent harness ([happy-agent](https://github.com/slopus/happy-agent)).

Install: Desktop DMG or `brew install --cask slopus/tap/happy`; CLI via npm; iOS, Android and web apps ([quick start](https://happy.engineering/quick-start/)). iOS app v1.8.0, iOS 15.1+ ([App Store](https://apps.apple.com/us/app/happy-codex-claude-code-app/id6748571505)).

First run

- Phone: welcome screen ("No email, no password"), then Create account or Restore. The account is a locally generated 32-byte secret with a base32 backup key ([security](https://happy.engineering/docs/security/)). Then a "Link Your Computer" checklist: install Happy, choose Link Mobile App, scan the QR, or paste the link ([en.ts translations in repo](https://github.com/slopus/happy)).
- CLI: `happy` asks Mobile App or Web, then prints a QR for `happy://terminal?<ephemeral X25519 pubkey>` and polls until the phone approves (`packages/happy-cli/src/ui/auth.ts`).
- Desktop: the app "starts its own agent runtime and finds the provider sign-ins. There is no key to paste." Then sign in to a model, pair the phone or skip, and a "Chief of Staff" agent helps import a project ([quick start](https://happy.engineering/quick-start/)).
- Prerequisite: one of Claude Code, Codex or Grok CLI already signed in (same page). `happy doctor` exists for diagnostics.
- iOS permissions declared: camera (QR), microphone, local network, calendar, notifications (`packages/happy-app/app.config.js` in [repo](https://github.com/slopus/happy)). When each prompt fires is not documented.

Agent connection

- CLI local mode spawns the real `claude` with inherited stdio, observes it with hooks and by tailing session JSONL. When a phone message arrives it kills the local process and switches to remote mode, which uses `@anthropic-ai/claude-agent-sdk` `query()` with `canUseTool` mapped to its permission handler (`claude/claudeLocalLauncher.ts`, `claude/claudeRemote.ts`, `claude/utils/permissionHandler.ts` in [repo](https://github.com/slopus/happy)).
- Codex: `codex app-server` over JSON-RPC on stdio, replacing an older MCP client (`codex/codexAppServerClient.ts`).
- Generic ACP: `happy acp -- <cmd>` (`agent/acp/*`).
- Happy Agent does not run the Claude Code or Codex binaries. It calls vendor APIs directly using the CLIs' stored credentials ([happy-agent README](https://github.com/slopus/happy-agent), [how it works](https://happy.engineering/how-it-works/)).

Agent auth

- CLI reuses the host's `claude` and `codex` logins and tells users to "Run claude auth login on the host machine" on failure (`claude/utils/providerAuth.ts`). Happy's own key is `~/.happy/access.key`.
- Happy Agent reads the Claude OAuth token from the Keychain item `Claude Code-credentials` (fallback `~/.claude/.credentials.json`) and `~/.codex/auth.json` ([happy-agent source](https://github.com/slopus/happy-agent)). This is the pattern Anthropic's policy forbids for third-party developers ([legal](https://code.claude.com/docs/en/legal-and-compliance)).

Workspaces: Desktop creates worktrees under `~/Happy/Workspaces` on branch `worktree/<name>` from a fresh `origin`. It copies `workspace.sync` files such as `.env` and runs `workspace.setup_commands`. Archive removes the worktree and keeps the branch ([workspaces](https://happy.engineering/workspaces/)). PR creation is not documented.

Approvals: CLI writes the request into encrypted session state, sends a push of kind `permission`, and resolves when the phone calls the `permission` RPC (`permissionHandler.ts`, `sources/sync/ops.ts`). Since Aug 2026 new sessions default to "Auto" (`packages/happy-app/CHANGELOG.md`). Desktop has four modes with Auto as default; Auto has "a separate read-only agent" judge boundary-crossing actions ([permissions](https://happy.engineering/permissions/)).

Mobile and remote

- Pairing: QR with an ephemeral NaCl box key; the phone returns the account key sealed to it; a signed challenge yields an auth token (`src/api/auth.ts`).
- Encryption: AES-256-GCM per session, keys wrapped with X25519 to an account key derived from the master secret ([encryption.md](https://github.com/slopus/happy/blob/main/docs/encryption.md)).
- Relay: Socket.IO to `api.cluster-fluster.com`, ciphertext stored in Postgres, self-hosting supported ([how it works](https://happy.engineering/docs/how-it-works/)). No LAN mode. The request for one has been open since Sep 2025 ([#101](https://github.com/slopus/happy/issues/101)).
- Push: the CLI sends through Expo's push service with plaintext titles and bodies such as "Permission request" and the session summary (`src/api/pushNotifications.ts`).
- Sleep: the daemon runs `caffeinate` and does not reconnect with the lid closed ([#1817](https://github.com/slopus/happy/issues/1817)).

Background process: the CLI auto-starts a detached daemon. `happy daemon install` needs sudo and writes a system LaunchDaemon at `/Library/LaunchDaemons/com.happy-cli.daemon.plist`, not a per-user agent (`daemon/mac/install.ts`). Happy Agent is started by Desktop; no launchd integration found.

Pain points: no push for permission in remote sessions ([#1383](https://github.com/slopus/happy/issues/1383)); pushes dropped ([#1511](https://github.com/slopus/happy/issues/1511)); relay down about 8 hours on 5 Oct 2026 ([#1866](https://github.com/slopus/happy/issues/1866)); the QR public key works as a permanent authenticator and requests never expire ([#1829](https://github.com/slopus/happy/issues/1829), [#674](https://github.com/slopus/happy/issues/674)); sending text while a permission is pending kills the request ([#1735](https://github.com/slopus/happy/issues/1735)); daemon sessions ignore `~/.claude/settings.json` ([#1396](https://github.com/slopus/happy/issues/1396)).

### 3.3 Omnara

Omnara's phone-control product is effectively dead, and the history is the useful part.

- v1 (Jul to Nov 2025): Python wrapper running `claude` in a PTY, screen-scraping "Do you want" prompts and typing the answer back ([PyPI 1.7.0](https://pypi.org/project/omnara/1.7.0/)). The README says the wrapper "became unfeasible to maintain with Claude Code's constant updates". Users saw it freeze after a Claude Code update ([#276](https://github.com/omnara-ai/omnara/issues/276)). Push went through Expo; no E2E claim.
- v2 (Nov 2025 to about Sep 2026): closed-source bun daemon on the Claude Agent SDK plus a Codex app-server adapter ([README at 500a82a](https://github.com/omnara-ai/omnara/blob/500a82ad85ba35d0a478810bb00e944ba93071b1/README.md)). On 11 Sep 2026 a maintainer said it would be deprecated and pointed users to paseo.sh and t3.codes ([#514](https://github.com/omnara-ai/omnara/issues/514)).
- Now (relaunched 2 Sep 2026): Go, Apache-2.0, "the open-source alternative to Claude Managed Agents" ([changelog](https://docs.omnara.com/changelog)). Its own agent loop, API keys only, approvals through a dashboard or Slack and Discord ([permissions](https://docs.omnara.com/tools/permissions)). The old iOS listing returns 404.
- The machine daemon `omnarad` installs with a curl script into `~/.omnarad` and "registers as a launchd user service on macOS" ([connect](https://docs.omnara.com/machines/connect)). It is the only competitor found that does this.

Pain points: tool approvals hung the dashboard ([#183](https://github.com/omnara-ai/omnara/issues/183)); 80+ pushes with auto-accept on ([#145](https://github.com/omnara-ai/omnara/issues/145)); browser auth failures in the CLI ([#279](https://github.com/omnara-ai/omnara/issues/279), [#285](https://github.com/omnara-ai/omnara/issues/285)).

### 3.4 Nimbalyst (formerly Crystal)

Crystal (stravu/crystal) was deprecated in Feb 2026; its last release v0.3.5 says "it will not be updated in the future" ([release](https://github.com/stravu/crystal/releases/tag/v0.3.5)). Notes on Crystal for history:

- Spawned `claude --output-format stream-json` and `codex exec --json` in node-pty ([claudeCodeManager.ts](https://github.com/stravu/crystal/blob/main/main/src/services/panels/claude/claudeCodeManager.ts)).
- Defaulted to `--dangerously-skip-permissions`. Its approve mode used `--permission-prompt-tool` with an MCP server bridged to an in-app dialog ([mcpPermissionServer.ts](https://github.com/stravu/crystal/blob/main/main/src/services/mcpPermissionServer.ts)).
- Put worktrees inside the repo without gitignoring them ([#152](https://github.com/stravu/crystal/issues/152)), never copied `.env` ([#39](https://github.com/stravu/crystal/issues/39)), and stored API keys in plain text ([#30](https://github.com/stravu/crystal/issues/30)).

Nimbalyst is the successor and an active, direct competitor. MIT, Electron, macOS, Windows, Linux, plus a native SwiftUI iOS app. v0.81.7 on 16 Oct 2026 ([repo](https://github.com/Nimbalyst/nimbalyst), [App Store](https://apps.apple.com/us/app/nimbalyst/id6756393105)).

- First run: DMG, Open Folder, choose Standard or Developer mode, tutorial, then enable Claude Agent or Codex in Settings > AI Providers. "You do not need to install a separate CLI." Claude login opens Terminal for `/login`, then the user clicks Refresh. Codex opens the ChatGPT browser flow ([Mac install guide](https://nimbalyst.com/docs/detailed-install-guides/mac-detailed-install-guide/)). The "are you a developer?" question kept reappearing ([#1069](https://github.com/Nimbalyst/nimbalyst/issues/1069)).
- Agents: Claude Agent SDK; Codex app-server over stdio (ACP for Codex is experimental); ACP for Copilot and Grok ([AI_PROVIDER_TYPES.md](https://github.com/Nimbalyst/nimbalyst/blob/main/docs/AI_PROVIDER_TYPES.md)). An ACP bug drops `session/request_permission` and hangs Copilot sessions ([#1658](https://github.com/Nimbalyst/nimbalyst/issues/1658)).
- Approvals: a per-project trust dialog with four levels (agent-verified, allow everything, edits only, ask every time). "Always" is written to `.claude/settings.local.json` ([AGENT_PERMISSIONS.md](https://github.com/Nimbalyst/nimbalyst/blob/main/docs/AGENT_PERMISSIONS.md)).
- Worktrees in `../{project}_worktrees/`, branch `worktree/<adjective-noun>` ([WORKTREES.md](https://github.com/Nimbalyst/nimbalyst/blob/main/docs/WORKTREES.md)).
- GitHub: "All GitHub authentication is delegated to the gh CLI" ([PullRequestHandlers.ts](https://github.com/Nimbalyst/nimbalyst/blob/main/packages/electron/src/main/ipc/PullRequestHandlers.ts)).
- Mobile: sign in on desktop (Google or magic link), sign in on iOS with the same account, then "Pair Device" with a QR. The phone can answer tool permissions, plan approvals and commit proposals. Pushes are routed so the phone stays quiet while the desktop is active ([mobile docs](https://nimbalyst.com/docs/mobile/mobile-app/)). Sync is E2E (AES-256-GCM), but push alert titles and Live Activity content leave the server in plaintext ([IDENTITY_AUTH_AND_ROOMS.md](https://github.com/Nimbalyst/nimbalyst/blob/main/docs/IDENTITY_AUTH_AND_ROOMS.md)).
- Background: the desktop host must be awake, and QR pairing assumes a display ([#416](https://github.com/Nimbalyst/nimbalyst/issues/416)). A headless `nimbalyst-node serve` is early ([packages/node](https://github.com/Nimbalyst/nimbalyst/blob/main/packages/node/README.md)).
- Pain points: login buttons failed on macOS Tahoe for lack of an Apple Events entitlement ([#57](https://github.com/Nimbalyst/nimbalyst/issues/57)); it ignored the macOS Keychain and read only `.credentials.json` ([#1470](https://github.com/Nimbalyst/nimbalyst/issues/1470)); iOS "Couldn't load sessions" ([#1733](https://github.com/Nimbalyst/nimbalyst/issues/1733)).

### 3.5 Claude Squad

Go TUI, AGPL-3.0, `brew install claude-squad` or a curl script that also installs tmux ([README](https://github.com/smtg-ai/claude-squad/blob/main/README.md), [install.sh](https://github.com/smtg-ai/claude-squad/blob/main/install.sh)). v1.0.20 on 20 Aug 2026.

- Runs any agent CLI interactively in tmux and screen-scrapes the pane to detect prompts ([tmux.go](https://github.com/smtg-ai/claude-squad/blob/main/session/tmux/tmux.go)). Auto-yes presses Enter when it sees the prompt text, and a background `--daemon` keeps doing it after `cs` exits ([daemon.go](https://github.com/smtg-ai/claude-squad/blob/main/daemon/daemon.go)).
- Worktrees in `~/.claude-squad/worktrees/`, branch `<user>/<session>`; no setup scripts, `.env` copying or ports ([#260](https://github.com/smtg-ai/claude-squad/issues/260)).
- Needs `gh` and checks `gh auth status`; pushes the branch but does not create a PR ([util.go](https://github.com/smtg-ai/claude-squad/blob/main/session/git/util.go)).
- First-run pain: "timed out waiting for tmux session" ([#132](https://github.com/smtg-ai/claude-squad/issues/132)); prompt sent before the CLI is ready ([#266](https://github.com/smtg-ai/claude-squad/issues/266)).

### 3.6 Vibe Kanban (sunsetting)

bloop shut down on 10 Apr 2026; the project is "community maintained" ([shutdown](https://www.vibekanban.com/blog/shutdown)). Apache-2.0. Rust backend; still a useful reference.

- Install: `npx vibe-kanban` downloads a prebuilt binary with a sha256 check and opens a browser; `--desktop` fetches a Tauri app ([download.ts](https://github.com/BloopAI/vibe-kanban/blob/main/npx-cli/src/download.ts)).
- First run: preferences (agent, IDE, notifications), optional GitHub or Google sign-in, then create an issue and a workspace ([getting-started.mdx](https://github.com/BloopAI/vibe-kanban/blob/main/docs/getting-started.mdx)). No prerequisite check; on macOS the first PR attempt offers to install `gh` via Homebrew ([github-integration.mdx](https://github.com/BloopAI/vibe-kanban/blob/main/docs/integrations/github-integration.mdx)).
- Agents: piped stdio, not a PTY. Claude runs as `npx -y @anthropic-ai/claude-code@<pinned> -p --output-format=stream-json --input-format=stream-json`, with `--permission-prompt-tool=stdio` when approvals are on ([claude.rs](https://github.com/BloopAI/vibe-kanban/blob/main/crates/executors/src/executors/claude.rs)). Codex runs `codex app-server` ([codex.rs](https://github.com/BloopAI/vibe-kanban/blob/main/crates/executors/src/executors/codex.rs)). Gemini, Qwen and Copilot use ACP ([acp/](https://github.com/BloopAI/vibe-kanban/tree/main/crates/executors/src/executors/acp)).
- Every default profile skips permissions ([default_profiles.json](https://github.com/BloopAI/vibe-kanban/blob/main/crates/executors/default_profiles.json)).
- Workspaces: worktrees in `$TMPDIR/vibe-kanban/worktrees`, branch `vk/<4hex>-<slug>`, setup, dev and cleanup scripts, "Copy Files" before setup, no port allocation ([projects-repositories.mdx](https://github.com/BloopAI/vibe-kanban/blob/main/docs/settings/projects-repositories.mdx), [browser-testing.mdx](https://github.com/BloopAI/vibe-kanban/blob/main/docs/browser-testing.mdx)).
- Remote: a pairing code on the host plus a web login at cloud.vibekanban.com ([remote-access.mdx](https://github.com/BloopAI/vibe-kanban/blob/main/docs/remote-access.mdx)). Whether it still works is unverified.
- Pain points: tried to install a pinned `claude-code@2.1.62` that does not exist ([#3347](https://github.com/BloopAI/vibe-kanban/issues/3347)); "Codex authentication required" blocked a user who does not use Codex ([#1374](https://github.com/BloopAI/vibe-kanban/issues/1374)); agents refused to start ([#1731](https://github.com/BloopAI/vibe-kanban/issues/1731)); old `gh` broke PR creation ([#1665](https://github.com/BloopAI/vibe-kanban/issues/1665)); local projects were lost after the cloud shut down ([#3354](https://github.com/BloopAI/vibe-kanban/issues/3354)).

### 3.7 Sculptor (Imbue)

Electron app with a Python backend, MIT since Jun 2026, v0.48.0 on 21 Sep 2026 ([repo](https://github.com/imbue-ai/sculptor), [release](https://github.com/imbue-ai/sculptor/releases/tag/sculptor-v0.48.0)). The imbue.com page is out of date ([imbue.com/sculptor](https://imbue.com/sculptor)).

- First run: "Your details" (name, email), then "Dependencies", which checks the Claude CLI and git, then "Add a repository" ([getting_started.md](https://github.com/imbue-ai/sculptor/blob/main/docs/help/getting_started.md)). The Claude CLI can be "managed by Sculptor or a custom binary" ([settings.md](https://github.com/imbue-ai/sculptor/blob/main/docs/help/settings.md)). The Claude install step once froze at 25% ([#14](https://github.com/imbue-ai/sculptor/issues/14)).
- Agent: `claude --dangerously-skip-permissions --permission-prompt-tool stdio --output-format=stream-json --input-format stream-json` ([process_manager_utils.py](https://github.com/imbue-ai/sculptor/blob/main/sculptor/sculptor/agents/default/claude_code_sdk/process_manager_utils.py)). It auto-approves every `can_use_tool` request ([output_processor.py](https://github.com/imbue-ai/sculptor/blob/main/sculptor/sculptor/agents/default/claude_code_sdk/output_processor.py)).
- It dropped Docker per agent for worktrees: "Most users found the extra isolation confusing", and Docker Desktop on macOS was "the wrong choice" ([history.md](https://github.com/imbue-ai/sculptor/blob/main/docs/history.md)). Containers cannot reach the macOS Keychain, so its container mode needs `claude setup-token` ([container_backend.md](https://github.com/imbue-ai/sculptor/blob/main/docs/help/experimental/container_backend.md)).
- Worktrees under `~/.sculptor/workspaces/<id>/code`, branch `<user>/<slug>`, a per-repo setup command ([workspaces.md](https://github.com/imbue-ai/sculptor/blob/main/docs/help/workspaces.md)). PRs via `gh`: "The first time, you may need to run gh auth login" ([pull_requests.md](https://github.com/imbue-ai/sculptor/blob/main/docs/help/pull_requests.md)).
- No mobile. Terminals died after macOS sleep ([#182](https://github.com/imbue-ai/sculptor/issues/182)).

### 3.8 Superset

Electron, Elastic License 2.0 (source-available), desktop-v1.38.0 on 14 Oct 2026 ([repo](https://github.com/superset-sh/superset), [LICENSE](https://github.com/superset-sh/superset/blob/main/LICENSE.md)). Free for local; Pro ($20/user/month) adds remote access and mobile ([pricing](https://superset.sh/pricing)).

- First run: a checklist with "live detection". It offers to install `gh` and can run `gh auth login` in-app, detects Claude Code and Codex, then asks to add a project ([install](https://docs.superset.sh/install)). Sign-in with GitHub or Google appears to be required, and a user asks to drop it ([#4894](https://github.com/superset-sh/superset/issues/4894)). Notification permission is needed for dock badges ([agent status](https://docs.superset.sh/agent-status)).
- Agents run in PTYs owned by a background `pty-daemon` ([pty-daemon](https://github.com/superset-sh/superset/tree/main/packages/pty-daemon)). Status comes from hooks and wrappers Superset installs into each agent's config under `~/.superset/bin` ([agent status](https://docs.superset.sh/agent-status)). No ACP. About 20 agents are built in, and custom agents need hooks "shaped like Claude Code's" ([add agent support](https://docs.superset.sh/add-agent-support)).
- Claude launches with `--dangerously-skip-permissions` by default ([Claude Code agent](https://docs.superset.sh/agents/claude-code)).
- Workspaces: worktree per branch, setup/teardown/run in `.superset/config.json`, `.env` copied by setup scripts ([setup scripts](https://docs.superset.sh/setup-teardown-scripts)). "Superset does not assign per-workspace port ranges" ([ports](https://docs.superset.sh/ports)).
- GitHub "fully supported via the gh CLI", `GH_TOKEN` for headless hosts ([FAQ](https://docs.superset.sh/faq), [host server](https://docs.superset.sh/cli/host-server)). No controlling TTY broke `gh` auth prompts ([#4775](https://github.com/superset-sh/superset/issues/4775)).
- Mobile: enable "Allow remote access to this device via relay", sign in to the same account on the phone. The desktop QR only opens the App Store ([remote access](https://docs.superset.sh/remote-access)). The relay is a Cloudflare Worker with JWT auth; no encryption code found ([relay src](https://github.com/superset-sh/superset/blob/main/apps/relay/src/index.ts)). "Push notifications are coming" ([FAQ](https://docs.superset.sh/faq)).
- Background: quitting the app stops the host service, so the phone loses reach; the workaround is `superset start --daemon` ([keep agents running](https://docs.superset.sh/keep-agents-running)). Agents "pause while the computer sleeps".
- Pain points: sign-in loops when the API is unreachable ([#7881](https://github.com/superset-sh/superset/issues/7881)); agent CLIs only on the `.zshrc` PATH are not found ([#7398](https://github.com/superset-sh/superset/issues/7398)); notification spam from subagents ([#6929](https://github.com/superset-sh/superset/issues/6929)).

### 3.9 cmux (manaflow-ai)

Native Swift terminal on libghostty, GPL-3.0 for the apps, v0.65.2 ([repo](https://github.com/manaflow-ai/cmux), [CHANGELOG](https://github.com/manaflow-ai/cmux/blob/main/CHANGELOG.md)). Not the smaller craigsc/cmux.

- First run: opens straight into a terminal, "no required account to use the terminal" ([getting started](https://cmux.com/docs/getting-started)). Agent hooks install with `cmux hooks setup` ([agent-hooks.md](https://github.com/manaflow-ai/cmux/blob/main/docs/agent-hooks.md)). Linking the CLI into `/usr/local/bin` may ask for an admin password.
- Agents run in plain PTYs. Integration is through hooks writing to a Unix socket. A Claude `PermissionRequest` hook parks for up to 120 s while the Feed panel, a notification action, or the phone answers ([feed.md](https://github.com/manaflow-ai/cmux/blob/main/docs/feed.md)).
- Mobile: TestFlight beta tied to a paid "Founders Edition". Sign in on Mac, enable "Enable iOS pairing", sign in on iOS, then Auto-Connect or Tailscale with a QR code. iOS asks for Local Network and Notifications ([iOS docs](https://cmux.com/docs/ios)). Transport design uses Iroh with relays and Bonjour ([iroh design](https://github.com/manaflow-ai/cmux/blob/main/docs/iroh-app-transport-architecture.md)). Since 0.65.0 (5 Oct 2026) "Push notifications and replies sent from the iPhone are end-to-end encrypted" ([CHANGELOG](https://github.com/manaflow-ai/cmux/blob/main/CHANGELOG.md), [phone-push-e2e.md](https://github.com/manaflow-ai/cmux/blob/main/docs/phone-push-e2e.md)).
- Sleep: "If the Mac sleeps, shuts down, or loses its connection, the phone cannot reach its terminals" ([iOS docs](https://cmux.com/docs/ios)).
- Pain points: pairing failures ([#12939](https://github.com/manaflow-ai/cmux/issues/12939), [#17498](https://github.com/manaflow-ai/cmux/issues/17498), [#12366](https://github.com/manaflow-ai/cmux/issues/12366)); expired relay TLS certificates broke all pairing on 14 Oct 2026 ([#20421](https://github.com/manaflow-ai/cmux/issues/20421)); users asking "how trust works and how the cmux account is used" ([#6085](https://github.com/manaflow-ai/cmux/issues/6085)); hooks broke Claude Code for a user ([#15058](https://github.com/manaflow-ai/cmux/issues/15058)).

### 3.10 Emdash

The closest architecture to the PRD. Apache-2.0, YC W26, v1.2.8 on 10 Oct 2026, no sign-in for local use ([repo](https://github.com/generalaction/emdash), [installation](https://emdash.com/docs/installation)).

- Onboarding: install an agent CLI (it detects 35), open or clone a project or connect over SSH, then optionally connect GitHub ([installation](https://emdash.com/docs/installation)).
- Agents: an ACP runtime with a per-session "permission broker" ([acp-runtime.md](https://github.com/generalaction/emdash/blob/main/agents/architecture/acp-runtime.md)), depending on `@agentclientprotocol/claude-agent-acp` and `@agentclientprotocol/codex-acp` ([package.json](https://github.com/generalaction/emdash/blob/main/packages/plugins/package.json)). ACP issues: worker crashes ([#3309](https://github.com/generalaction/emdash/issues/3309)), a Claude ACP restore loop ([#3260](https://github.com/generalaction/emdash/issues/3260)).
- Workspaces: worktree per task in `~/emdash/worktrees`, "preserve patterns" for `.env`, setup/run/teardown in `.emdash.json`, `EMDASH_PORT` as the base of a 10-port range ([project config](https://emdash.com/docs/project-config)).
- GitHub: Emdash account, import from `gh`, or device flow; a user asked it not to use `gh` without permission ([GitHub accounts](https://emdash.com/docs/github-accounts), [#1704](https://github.com/generalaction/emdash/issues/1704)).
- No mobile.

### 3.11 Terragon

Shut down: "As of February 9th, 2026, the service has been shut down" ([terragonlabs.com](https://www.terragonlabs.com)). Snapshot in [terragon-oss](https://github.com/terragon-labs/terragon-oss). Skipped.

### 3.12 Claude Code (Anthropic)

Versions checked: Claude Code 2.1.301 (15 Oct 2026) ([release](https://github.com/anthropics/claude-code/releases/tag/v2.1.301)).

Remote Control ([docs](https://code.claude.com/docs/en/remote-control))

- Connects claude.ai/code or the Claude iOS and Android apps to a local `claude` process. Added in v2.1.51 (24 Feb 2026) ([CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)).
- Requires Pro, Max, Team or Enterprise. "API keys are not supported."
- Pairing: the user is already signed in to the same claude.ai account. Server mode prints a URL and shows a QR on spacebar. Optional "Trusted Devices".
- `--spawn worktree` gives each on-demand session its own worktree. `--capacity` defaults to 32.
- "All traffic travels through the Anthropic API over TLS." While connected, "the session transcript ... is stored on Anthropic servers."
- Push: `/config` toggles "Push when Claude decides" and "Push when actions required" (permission prompts and questions). Permission prompts stay open until answered.
- Sleep: reconnects after sleep, but "your computer has to stay on and the claude process has to keep running".

Desktop app Code tab ([desktop](https://code.claude.com/docs/en/desktop)): sign in, pick environment (Local, Cloud, SSH, WSL), folder, model and permission mode. Per-session worktrees in `<project>/.claude/worktrees/`, `.worktreeinclude` for gitignored files. OS notification when a session finishes. Dispatch sends a phone push "when it finishes or needs your approval".

Driving Claude Code from another app

- Claude Agent SDK, "a library that runs the Claude Code binary" ([overview](https://code.claude.com/docs/en/agent-sdk/overview)). `canUseTool` "can stay pending indefinitely" ([user input](https://code.claude.com/docs/en/agent-sdk/user-input)).
- If no permission mode is set, the default "can be auto mode", and auto-approved calls never reach `canUseTool` ([permissions](https://code.claude.com/docs/en/agent-sdk/permissions)).
- Hooks: `PermissionRequest` can allow or deny; the `Notification` matcher `permission_prompt` fires after about 6 s ([hooks](https://code.claude.com/docs/en/hooks)).
- Headless: `claude -p --output-format stream-json`, `--permission-prompt-tool`. `--bare` skips OAuth and Keychain and needs an API key; the docs say it "will become the default for -p in a future release" ([headless](https://code.claude.com/docs/en/headless)).

Auth policy ([legal and compliance](https://code.claude.com/docs/en/legal-and-compliance))

- "Anthropic does not permit third-party developers to offer Claude.ai login into their own applications, or to route requests through Free, Pro, or Max plan credentials on behalf of their users."
- "developers may not collect, store, or intermediate Claude.ai credentials or session tokens". The same sentence says sign-in "must complete through Anthropic's own flow".
- It does not "prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription".
- Shipping Claude Code inside a product requires the Commercial Terms and an unmodified binary, with no auth method removed.
- Billing: Agent SDK and third-party usage still draw from subscription limits after Anthropic paused a planned change on 15 Jun 2026 ([help center](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)).

### 3.13 OpenAI Codex

- CLI install by curl script, npm or Homebrew. `codex login` (browser or device code) or `codex login --with-api-key`. Credentials in `~/.codex/auth.json` or the OS keyring, controlled by `cli_auth_credentials_store` ([auth](https://learn.chatgpt.com/docs/auth.md)).
- The Codex app shipped on macOS on 2 Feb 2026 and merged into the ChatGPT desktop app on 9 Jul 2026 ([what's new](https://learn.chatgpt.com/docs/whats-new.md)). Per-chat worktrees in detached HEAD with "Handoff" to local ([worktrees](https://learn.chatgpt.com/docs/environments/git-worktrees.md)).
- Mobile: Settings > Connections > Control this Mac or PC, then scan a QR with the ChatGPT phone app. Setup has to start from the desktop app, not the CLI. Phone can approve commands and review diffs. "If that computer sleeps, loses network access, or closes the app, remote access stops." Traffic goes through "a secure relay layer" ([mobile](https://learn.chatgpt.com/docs/mobile.md), [remote connections](https://learn.chatgpt.com/docs/remote-connections.md)).
- app-server is "the interface Codex uses to power rich clients": JSON-RPC over stdio JSONL. Approvals arrive as server requests such as `item/commandExecution/requestApproval` with `accept`, `acceptForSession`, `decline`, `cancel` ([app-server](https://learn.chatgpt.com/docs/app-server.md)). `codex mcp-server` is gone ([SDK](https://learn.chatgpt.com/docs/codex-sdk.md)).
- Auth policy for app-server: local or open-source apps may keep using it; "App-server authentication has never been permitted for commercial or hosted services". Paid apps should use Sign in with ChatGPT ([app-server](https://learn.chatgpt.com/docs/app-server.md), [SIWC](https://developers.openai.com/siwc/token-sharing-open-source)).
- `approval_policy = "untrusted"` is retired and "can prevent either client from starting"; use per-project `trust_level` ([approvals](https://learn.chatgpt.com/docs/agent-approvals-security.md)). `notify` fires only on turn completion ([config](https://learn.chatgpt.com/docs/config-file/config-advanced.md)).

### 3.14 Zed and ACP

- ACP is JSON-RPC over stdio for local agents ([intro](https://agentclientprotocol.com/overview/introduction)). `session/request_permission` offers options of kind `allow_once`, `allow_always`, `reject_once`, `reject_always`. The client must answer `cancelled` on turn cancel. No timeout is specified ([tool calls](https://agentclientprotocol.com/protocol/tool-calls)).
- Zed installs agents from the ACP Registry (launched 28 Jan 2026), which pins exact npx versions: `@agentclientprotocol/claude-agent-acp@0.92.0` and `@agentclientprotocol/codex-acp@2.2.2` at time of writing ([blog](https://zed.dev/blog/acp-registry), [claude-acp](https://github.com/agentclientprotocol/registry/blob/main/claude-acp/agent.json), [codex-acp](https://github.com/agentclientprotocol/registry/blob/main/codex-acp/agent.json)). Zed refreshes the registry at most hourly ([agent_registry_store.rs](https://github.com/zed-industries/zed/blob/main/crates/project/src/agent_registry_store.rs)). Custom agents go in `agent_servers` ([external agents](https://zed.dev/docs/ai/external-agents)).
- Renames: `@zed-industries/claude-code-acp` is deprecated, "renamed to @agentclientprotocol/claude-agent-acp" ([npm](https://registry.npmjs.org/@zed-industries/claude-code-acp)). `zed-industries/codex-acp` is archived: "The new adapter is built on the new Codex App Server" ([repo](https://github.com/zed-industries/codex-acp)). The new [codex-acp](https://github.com/agentclientprotocol/codex-acp) is TypeScript and needs Node 22+.
- claude-agent-acp pins `@anthropic-ai/claude-agent-sdk`, which embeds the CLI. A maintainer: "you can't just connect existing Claude CLI via ACP" ([#517](https://github.com/agentclientprotocol/claude-agent-acp/issues/517)). Five releases shipped between 10 and 16 Oct 2026 ([releases](https://github.com/agentclientprotocol/claude-agent-acp/releases)).
- Auth: registry agents must offer "Agent Auth" or "Terminal Auth" ([AUTHENTICATION.md](https://github.com/agentclientprotocol/registry/blob/main/AUTHENTICATION.md)). Zed tells users to run `/login` in a Claude thread ([external agents](https://zed.dev/docs/ai/external-agents)). JetBrains launches the adapter with `--hide-claude-auth` because it "is not allowed to distribute Claude Code version that's logging in via Claude subscription" ([#517](https://github.com/agentclientprotocol/claude-agent-acp/issues/517)). JetBrains Air says a Claude agent added through `acp.json` "can't run on a Claude Free, Pro, or Max subscription" ([Air docs](https://www.jetbrains.com/help/air/select-agents-and-models.html)).
- Issues that matter for Swarn:
  - Spawned as a child, codex-acp hit "Authentication required" because of macOS Keychain access ([zed codex-acp #310](https://github.com/zed-industries/codex-acp/issues/310)).
  - claude-agent-acp gets "no macOS security session inherited" ([#548](https://github.com/agentclientprotocol/claude-agent-acp/issues/548)).
  - claude-agent-acp sends `request_permission` while in bypass mode ([#1196](https://github.com/agentclientprotocol/claude-agent-acp/issues/1196)) and deadlocks on permission ID desync with background subagents ([#851](https://github.com/agentclientprotocol/claude-agent-acp/issues/851)).
  - codex-acp ignores config.toml approval policies ([#310](https://github.com/agentclientprotocol/codex-acp/issues/310)), has no mode that asks before every command ([#555](https://github.com/agentclientprotocol/codex-acp/issues/555)), and returns InternalError instead of AuthRequired on expired ChatGPT credentials ([#495](https://github.com/agentclientprotocol/codex-acp/issues/495)).
- License mismatch: claude-agent-acp is Apache-2.0 in its repo, but the registry entry says "proprietary" ([agent.json](https://github.com/agentclientprotocol/registry/blob/main/claude-acp/agent.json)).

### 3.15 Others checked briefly

- JetBrains Air: install, sign in to an agent provider (skippable for ACP-only use), trust the project, then run tasks in workspace, worktree, Docker or cloud ([quickstart](https://www.jetbrains.com/help/air/quickstart-air-app.html), [run tasks](https://www.jetbrains.com/help/air/run-tasks.html)). Mobile is "coming soon".
- claude-remote-approver: a `PermissionRequest` hook that posts to an ntfy.sh topic with Approve and Deny buttons ([repo](https://github.com/yuuichieguchi/claude-remote-approver)). It shows how cheap a basic phone approval is. Its ntfy topics are not encrypted by default.
- VibeTunnel, opcode, Agent Deck and Sidecar look stale or are out of scope. Warp, Cursor, Jules and Factory are cloud agents or editors.

## 4. Implications for Swarn

### 4.1 First-run checklist, in order

Each step names the evidence and the PRD section it touches.

1. Install and register the daemon. The desktop app should register `swarnd` as a per-user launchd agent on first launch, with no Terminal step. Competitors without a helper lose sessions on quit (Conductor), and those with a hand-rolled helper get it wrong: Happy needs a sudo LaunchDaemon, and Superset's host dies on quit. Omnara's `omnarad` launchd user service is the only clean example. My suggestion, not taken from any competitor: register through `SMAppService` so the user sees one Login Items approval ([Apple docs](https://developer.apple.com/documentation/servicemanagement/smappservice)). This answers PRD §10 open item 4 with one code path. Keep `swarn daemon install` for headless use.
2. Capture the user's environment for the daemon. A launchd agent gets a minimal `PATH` and no login shell. Conductor captures an interactive login shell's env per workspace with a 5 s timeout ([shells](https://www.conductor.build/docs/reference/shells)). Superset broke when agent CLIs were only on the `.zshrc` PATH ([#7398](https://github.com/superset-sh/superset/issues/7398)). Do this once at install, show the result, and re-run on demand. PRD §1.1, §4.3.
3. Check that the daemon can authenticate agents. This is the riskiest step. Agents spawned as children of another process have failed on Keychain access ([zed codex-acp #310](https://github.com/zed-industries/codex-acp/issues/310), [claude-agent-acp #548](https://github.com/agentclientprotocol/claude-agent-acp/issues/548)), and Conductor fixed a Keychain auth false negative ([page 3](https://www.conductor.build/changelog/page/3)). Run a real `claude auth status` and `codex login status` from inside `swarnd` under launchd, not from the app. Add this to the week 1 measurements in PRD §8.
4. Detect and sign in to each agent, one card per agent, skippable. Do not block on an agent the user does not use (Vibe Kanban [#1374](https://github.com/BloopAI/vibe-kanban/issues/1374)). For Claude, open an embedded or system terminal running `claude /login`, as Conductor and Nimbalyst do, and poll status. Swarn must not show its own Claude login or read the token (Anthropic [policy](https://code.claude.com/docs/en/legal-and-compliance)). Warn when `ANTHROPIC_API_KEY` is set, because it overrides the subscription ([Conductor](https://www.conductor.build/docs/reference/harnesses/claude-code)). For Codex, run `codex login`. API key mode is the fallback for both. PRD §4.2.
5. Pick agent binaries. Default to a Swarn-pinned, tested version (Conductor bundles pinned binaries), with a visible "use the one on my PATH" switch. Never resolve a version from npm at first run (Vibe Kanban [#3347](https://github.com/BloopAI/vibe-kanban/issues/3347)). Record the version so the PRD §9.2 ACP contract tests know which transcript applies.
6. Check GitHub. Run `gh --version` and `gh auth status` here instead of only at daemon startup. Check for an unaccepted Xcode license (Conductor 0.36.7, [page 8](https://www.conductor.build/changelog/page/8)). If `gh` is missing, offer the Homebrew install (Superset, Vibe Kanban) or a `GH_TOKEN` paste (Conductor 0.25.5). Make GitHub skippable until the first PR, since `gh` is only needed for PRs. PRD §4.4.
7. Add a repository. Accept an existing local repo first; cloning can come later. Put worktrees outside the repo (Crystal [#152](https://github.com/stravu/crystal/issues/152); Conductor moved off nested `.conductor` folders). PRD §4.3.
8. Configure the repo: setup script, run script, archive script, files to copy. Read `.worktreeinclude` and default to `.env*`, as Conductor does; Claude Code uses the same file. Let the setup script fail without blocking the first prompt (Conductor 0.57.0). Prefilling a draft config from the repo's lockfiles is optional for v0.1. PRD §4.3.
9. Set the approval policy before the first session. Show a short trust choice (Nimbalyst has four levels), with "ask me for writes and commands" as the default. Write it to `~/.config/swarn/policy.toml`. Pass an explicit permission mode to the adapter, because the Claude SDK default "can be auto mode" and auto-approved calls never reach `canUseTool` ([permissions](https://code.claude.com/docs/en/agent-sdk/permissions)). PRD §2.6.
10. Request macOS notification permission for desktop alerts, with the reason stated (Conductor 0.45.0 notifies on tool permission requests). Explain the Files and Folders prompts that will name Swarn or `swarnd` when an agent reads Downloads or Desktop, and offer Full Disk Access as an option, not a requirement (Conductor [security](https://www.conductor.build/docs/reference/security-and-permissions)).
11. Offer keep-awake while agents run. Every phone story fails on sleep (Codex, Claude Remote Control, cmux, Superset, Happy [#1817](https://github.com/slopus/happy/issues/1817)). Conductor and Happy both use caffeinate. Say plainly what happens with the lid closed. PRD §1.3 already says the v0.1 phone gets nothing while the daemon sleeps.
12. Pair the phone, optional and skippable. Show the QR, and fire the iOS camera, Local Network and Notifications prompts during pairing, in that order. cmux and Happy ask for the same set, and PRD §2.7 already wants the Local Network prompt at pairing. Offer "Tailscale found" or "LAN only" status on the Mac side before showing the QR, since Swarn has no relay in v0.1. PRD §2.5, §2.7.
13. Send a test approval end to end, from a fake permission request in `swarnd` to a push and back to the daemon. Competitors' top phone complaints are pushes that never arrive (Happy [#1383](https://github.com/slopus/happy/issues/1383), [#1511](https://github.com/slopus/happy/issues/1511)) and pairing that silently fails (cmux [#12939](https://github.com/manaflow-ai/cmux/issues/12939)). The fake ACP agent in PRD §9.2 can drive this.
14. Start the first workspace with a prompt. Conductor opens straight into a composer (Dispatcher, 0.63.0).

### 4.2 Where competitor evidence conflicts with the PRD

- PRD §0, §1.5, §4.2 assume Zed's ACP adapters. The Codex adapter is now a Node package on top of `codex app-server`, so Swarn would run Rust daemon, then Node, then Codex. OpenAI calls app-server the interface for rich clients, and Happy, Nimbalyst and Vibe Kanban already talk to it directly. Consider a native app-server client for Codex and keep ACP for Claude, or accept the Node dependency and bundle a Node runtime. Either way, update the package names in the PRD.
- PRD §4.2 assumes the adapter uses the user's Claude Code. claude-agent-acp embeds its own SDK and CLI version, and it has shipped five releases in one week. Pin it and treat upgrades as deliberate, which matches the §9.2 contract tests.
- PRD §4.2 and the auth policy. Swarn as an open-source local app that launches the unmodified CLI under the user's own login matches what Anthropic allows. Using claude-agent-acp, which embeds the SDK, is a grey area: Zed ships it with subscription login, JetBrains hides it, and the maintainers reverted a change ([#723](https://github.com/agentclientprotocol/claude-agent-acp/pull/723)). Get written guidance from Anthropic before launch, and never add a Swarn-hosted Claude login. For Codex, app-server auth is fine for an open-source local app and not allowed for a commercial one ([app-server](https://learn.chatgpt.com/docs/app-server.md)).
- PRD §4.4 runs the `gh` preflight only at daemon startup. Conductor's history shows `gh` detection is the most fragile part of onboarding. Move the preflight into onboarding, add the Xcode license check, and consider `GH_TOKEN` as a fallback. This also helps PRD §10 open item 2.
- PRD §10 open item 4 (who starts the daemon). Evidence favors the app registering a per-user launchd agent on first run. Of the five products with a helper, the ones that left it optional or manual (Happy, Superset) have the most "phone can't reach Mac" reports.
- PRD §4.3 allocates one port per workspace (`SWARN_PORT`, range 31000-31999). Conductor and Emdash both hand out 10-port ranges because apps often need a second port. Consider `SWARN_PORT` as the base of a 10-port block.
- PRD §4.3 does not mention copying gitignored files. Conductor, Vibe Kanban, Happy and Emdash all copy `.env*` by default or by pattern, and users of tools that did not ask for it repeatedly (Crystal [#39](https://github.com/stravu/crystal/issues/39), Claude Squad [#260](https://github.com/smtg-ai/claude-squad/issues/260)). Add `.worktreeinclude` support.
- PRD §2.6 expiry. Claude Remote Control keeps prompts open until answered, the SDK's `canUseTool` can wait indefinitely, and ACP sets no timeout. The PRD's 30 minute default is stricter than the vendors', which is fine, but expiring must send an explicit deny or cancel to the adapter. Conductor shipped a bug where "local command approval timeouts" were "reported as user denials" (0.83.0, [page 2](https://www.conductor.build/changelog/page/2)).
- PRD §2.6 vs Happy [#1735](https://github.com/slopus/happy/issues/1735). A text reply while a permission is pending killed the request in Happy. Decide what Swarn does when the phone sends a reply instead of a decision.
- PRD §1.3 and the differentiation claim. Every competitor routes the phone through a relay, and most push plaintext titles (Happy, Nimbalyst). cmux now encrypts pushes, so "E2E encrypted push" alone no longer sets Swarn apart. "No vendor relay, phone talks to your Mac directly, and pushes are encrypted" does, until v0.2 adds a relay.

## 5. Sources

Conductor

- https://www.conductor.build/docs/
- https://www.conductor.build/docs/installation
- https://www.conductor.build/docs/first-workspace
- https://www.conductor.build/docs/configure-your-project
- https://www.conductor.build/docs/faq
- https://www.conductor.build/docs/troubleshooting/issues
- https://www.conductor.build/docs/reference/security-and-permissions
- https://www.conductor.build/docs/reference/privacy
- https://www.conductor.build/docs/reference/harnesses
- https://www.conductor.build/docs/reference/harnesses/claude-code
- https://www.conductor.build/docs/reference/harnesses/codex
- https://www.conductor.build/docs/guides/providers
- https://www.conductor.build/docs/concepts/git-worktrees
- https://www.conductor.build/docs/concepts/workspaces-and-branches
- https://www.conductor.build/docs/reference/scripts
- https://www.conductor.build/docs/reference/files-to-copy
- https://www.conductor.build/docs/reference/environment-variables
- https://www.conductor.build/docs/reference/shells
- https://www.conductor.build/docs/reference/cities
- https://www.conductor.build/docs/reference/settings/reference
- https://www.conductor.build/docs/reference/settings/managed
- https://www.conductor.build/docs/guides/review-and-merge
- https://www.conductor.build/docs/cloud
- https://www.conductor.build/pricing
- https://www.conductor.build/changelog and pages 2 to 21 (https://www.conductor.build/changelog/page/N)
- https://www.conductor.build/changelog/0.90.0-conductor-for-ios
- https://www.conductor.build/changelog/0.89.0-sign-in-with-chatgpt
- https://www.conductor.build/changelog/0.41.0-tool-approval-codex-plan-mode-beta
- https://www.conductor.build/changelog/0.36.3-submit-a-prompt-sdk-memory-leaks
- https://www.conductor.build/changelog/0.52.0-sounds-colors
- https://www.conductor.build/changelog/0.57.0-sync-agent-configs
- https://www.conductor.build/changelog/0.63.0-cursor-support-dispatcher
- https://apps.apple.com/us/app/conductor-build/id6791228564

Happy

- https://github.com/slopus/happy, https://github.com/slopus/happy/releases
- https://github.com/slopus/happy-desktop, https://github.com/slopus/happy-agent
- https://github.com/slopus/slopus.github.io
- https://happy.engineering/quick-start/, https://happy.engineering/docs/security/, https://happy.engineering/how-it-works/, https://happy.engineering/workspaces/, https://happy.engineering/permissions/, https://happy.engineering/mobile/, https://happy.engineering/models/
- https://github.com/slopus/happy/blob/main/docs/encryption.md
- https://apps.apple.com/us/app/happy-codex-claude-code-app/id6748571505
- Issues: #101, #674, #1383, #1396, #1511, #1735, #1817, #1829, #1866 on https://github.com/slopus/happy/issues

Omnara

- https://github.com/omnara-ai/omnara, https://pypi.org/project/omnara/1.7.0/
- https://docs.omnara.com/changelog, https://docs.omnara.com/machines/connect, https://docs.omnara.com/tools/permissions
- Issues: #145, #183, #276, #279, #285, #514 on https://github.com/omnara-ai/omnara/issues

Crystal and Nimbalyst

- https://github.com/stravu/crystal, https://github.com/stravu/crystal/releases/tag/v0.3.5
- https://github.com/Nimbalyst/nimbalyst and its docs folder
- https://nimbalyst.com/docs/detailed-install-guides/mac-detailed-install-guide/, https://nimbalyst.com/docs/mobile/mobile-app/
- https://apps.apple.com/us/app/nimbalyst/id6756393105
- Issues: Crystal #30, #39, #152; Nimbalyst #57, #416, #1069, #1470, #1658, #1733

Claude Squad

- https://github.com/smtg-ai/claude-squad and source files linked above
- Issues: #132, #260, #266

Vibe Kanban

- https://github.com/BloopAI/vibe-kanban and docs folder, https://www.vibekanban.com/blog/shutdown
- Issues: #1374, #1665, #1731, #3347, #3354

Sculptor

- https://github.com/imbue-ai/sculptor and docs/help folder, https://imbue.com/sculptor
- Issues: #14, #182

Superset

- https://github.com/superset-sh/superset, https://superset.sh/pricing
- https://docs.superset.sh/install, /agent-status, /add-agent-support, /agents/claude-code, /setup-teardown-scripts, /ports, /faq, /remote-access, /keep-agents-running, /cli/host-server
- Issues: #4775, #4894, #6929, #7398, #7881

cmux

- https://github.com/manaflow-ai/cmux, CHANGELOG.md, docs/feed.md, docs/agent-hooks.md, docs/phone-push-e2e.md, docs/iroh-app-transport-architecture.md
- https://cmux.com/docs/getting-started, https://cmux.com/docs/ios
- Issues: #6085, #12366, #12939, #15058, #17498, #20421

Emdash

- https://github.com/generalaction/emdash, https://emdash.com/docs/installation, /project-config, /github-accounts
- Issues: #1704, #3260, #3309

Terragon

- https://www.terragonlabs.com, https://github.com/terragon-labs/terragon-oss

Claude Code

- https://code.claude.com/docs/en/remote-control
- https://code.claude.com/docs/en/desktop
- https://code.claude.com/docs/en/legal-and-compliance
- https://code.claude.com/docs/en/agent-sdk/overview, /agent-sdk/user-input, /agent-sdk/permissions
- https://code.claude.com/docs/en/hooks, https://code.claude.com/docs/en/headless
- https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan

OpenAI Codex

- https://learn.chatgpt.com/docs/auth.md, /whats-new.md, /mobile.md, /remote-connections.md, /environments/git-worktrees.md, /app-server.md, /codex-sdk.md, /agent-approvals-security.md, /config-file/config-advanced.md
- https://developers.openai.com/siwc/token-sharing-open-source

Zed and ACP

- https://agentclientprotocol.com/overview/introduction, https://agentclientprotocol.com/protocol/tool-calls
- https://zed.dev/blog/acp-registry, https://zed.dev/docs/ai/external-agents
- https://github.com/agentclientprotocol/registry (claude-acp/agent.json, codex-acp/agent.json, AUTHENTICATION.md)
- https://github.com/agentclientprotocol/claude-agent-acp (issues #517, #548, #851, #1196; PR #723)
- https://github.com/agentclientprotocol/codex-acp (issues #310, #495, #555)
- https://github.com/zed-industries/codex-acp (archived; issue #310)
- https://registry.npmjs.org/@zed-industries/claude-code-acp
- https://github.com/zed-industries/zed/blob/main/crates/project/src/agent_registry_store.rs

Others

- https://www.jetbrains.com/help/air/select-agents-and-models.html, https://www.jetbrains.com/help/air/quickstart-air-app.html
- https://github.com/yuuichieguchi/claude-remote-approver
- https://developer.apple.com/documentation/servicemanagement/smappservice
