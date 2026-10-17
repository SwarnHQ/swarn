# Orca: how it's built, and what Swarn takes from it

Orca (`stablyai/orca`, MIT, about 95k stars) runs Claude Code, Codex, OpenCode and other agents side by side in git worktrees. It's an Electron and React desktop app for macOS, Windows and Linux, with an Expo phone app and a relay on GCP. This note comes from reading commit `05086a89` (16 Oct 2026). Nothing from the repo was run. Paths are relative to the repo root. Section numbers like §5.7 point to `docs/TECHNICAL_PRD.md`.

The codebase is very large. `src/main/runtime` alone has about 1,200 files, there are about 720 RPC methods, and most files carry "Why:" comments. A lot of what follows is good design buried under defensive layers. The goal here is to take the ideas, not the size.

## 1. Shape

- **Electron main** hosts the runtime: projects, worktrees, git, and structured agent sessions (`src/main/runtime/`).
- **A detached terminal daemon** (`src/main/daemon/`) owns every PTY, so terminal agents survive quitting the app. On macOS it runs from a signed helper app copied out of the bundle and started as a non-persistent launchd job, so permission grants survive updates (`docs/reference/macos-daemon-tcc-attribution.md`).
- **`orcad`** is the same runtime without Electron, used for `orca serve`, SSH hosts and cloud VMs. A CI check fails if anything in the runtime imports `electron`.
- **Structured chat sessions are children of Electron main**, not of the daemon. They stop when the app quits and are offered for resume on the next launch (`agent-session-recovery.json`). Swarn's daemon owns its agents outright, which Orca only achieves for terminal sessions.

## 2. Driving agents

| Agent | How |
| --- | --- |
| Claude Code | `@anthropic-ai/claude-agent-sdk` 0.3.284 `query()` with streaming input, pointed at the user's installed `claude`. `canUseTool` makes the SDK add `--permission-prompt-tool stdio` (`src/main/claude/claude-stream-json-connection.ts:139`). |
| Codex | `codex app-server` over JSON-RPC: `thread/start` or `thread/resume`, `turn/start`, `turn/steer`, `turn/interrupt`, `thread/revert`, `model/list`. |
| Grok, OpenCode, OMP | ACP, with per-vendor "dialects" for extensions. Protocol types are generated from a pinned ACP schema and checked in CI. |
| Others | Terminal only, observed through hooks and the agents' own JSONL transcripts. |

Points worth keeping:

- **Every option is stated on every start and resume.** Codex otherwise inherits approval policy from `config.toml` or from the thread's last run. Flags the user saved that Orca owns are stripped.
- **Changing permission mode restarts the child before the next send.** Agents read their mode only at start.
- **"Completed" doesn't mean success.** Both CLIs report API errors as finished turns, so Orca records a turn's outcome separately from its state.
- **Big tool output is cut at the host.** It keeps a 16 KB head plus byte length, a SHA-256 and a `truncated` flag, and the truncation is shown.
- **Children get a spawn token in their environment** (`ORCA_AGENT_SESSION_SPAWN_TOKEN`), so orphan detection survives PID reuse. "Can't tell" is never treated as "dead".

## 3. The journal

Structured chats are stored in one SQLite file as *items with revisions*, not as a stream of deltas (`src/shared/agent-session-journal-types.ts`):

```ts
type AgentJournalItemBody =
  | {kind:'message', role, blocks, state?}            // user, assistant, reasoning, system
  | {kind:'tool-call', name, input, state, output?}   // output is the bounded payload
  | {kind:'diff', path, patch}
  | {kind:'approval', title, description?, options, resolution}
  | {kind:'question', questions, resolution}
  | {kind:'status', text, tone?, failure?}
  | {kind:'turn', turnId, state, outcome?, startedAt, completedAt, durationMs, contextUsage?}
type AgentJournalRenderItem = { itemId, revision, body, sequence, observedAt }
```

- **Streamed text is coalesced for 60 ms into snapshot rows** holding the full text so far. Any lifecycle event flushes the window first. Applying a batch twice converges, so replay and reconnect are simple.
- **A user's message is written before it is sent** (a `submission` row). That row is also the optimistic bubble. Its dispatch state goes `pending`, then `accepted`, `rejected` or `unknown`, and `unknown` is never retried automatically.
- **Answering an approval is a compare-and-set** on the item's resolution, made while holding the live callback. Two clients can race and exactly one wins.
- **Backpressure reaches the agent.** When the journal write queue passes 16 MB, the reader on the agent's stdout pauses.
- **Messages sent while the agent is busy are queued** in their own table. They show as cards above the composer, where they can be edited or deleted, and drain in order when the turn ends. Stop pauses the queue. "Steer" sends one into the running turn: Claude takes it through the input stream, Codex through `turn/steer`.

## 4. The chat UI

From `src/renderer/src/components/native-chat` (about 430 non-test files) and `src/shared/native-chat-*`.

- **Tool calls fold into runs.** An unbroken stretch of tool calls between two pieces of prose becomes one row whose header is a sentence: "Read 7 files, ran 17 commands, and searched 4 times" (`native-chat-tool-run-sentence.ts`). While live, the header shows the latest call. One element serves the run's whole life, so it never remounts. Each tool category has a fixed icon that doesn't change with status.
- **A finished turn folds** behind "Worked for 1m 12s ›" and shows only its final answer (`native-chat-turn-fold.ts`).
- **Approval and question cards take the composer's place** instead of sitting in the transcript. Number keys pick an option and Esc collapses the card. Once answered, the card leaves a receipt row in the transcript.
- **Batching on the client.** Only assistant text is batched (48 ms). Tool calls, approvals and queue changes apply at once.
- **Rows keep their identity.** Unchanged items keep their rendered objects through a WeakMap cache, and tests count per-row work instead of renders.
- **Markdown streams cheaply.** It's split into top-level blocks and only the growing last block is re-parsed, after a pass that closes unterminated syntax.
- **Text is revealed at a steady pace,** word by word, with a deadline per arrival. This smooths bursty streams at the cost of up to 350 ms of deliberate lag.
- **Scrolling and the tail:**
  - Following the tail stops only on a reader's gesture (wheel up, PageUp, scrollbar), never because layout moved.
  - Programmatic scrolls are marked so their echo isn't mistaken for the reader.
  - A "Jump to latest" pill appears when the reader is more than 48 px from the bottom.
  - A "New" marker shows where unseen content starts.
  - A rail on the right edge has one tick per user message, with a preview on hover.
- **The composer:**
  - `/` commands and `@` files.
  - Image paste and drop.
  - Drafts saved per chat.
  - Up arrow walks back through sent prompts.
  - Model and effort pickers.
  - A context-usage ring.
  - The send button becomes Stop, and ignores a double click so it can't cancel the turn it just started.
- **One state function for the whole view:** loading, error, empty, or ready. Any message beats loading or empty, so an optimistic send never flashes away. The loading spinner waits 250 ms before it appears. Reconnects back off from 750 ms to 30 s with the transcript still on screen.
- **Typography scales from one variable.** The chat's type scale and spacing derive from a single font-size variable.

## 5. Phone, relay and pairing

- **The phone app** is Expo and React Native. Much of its UI is a web bundle *downloaded from the paired desktop*, so the phone's UI always matches the desktop's version (`src/main/runtime/rpc/methods/mobile-web-bundle.ts`).
- **Pairing** works by QR code (`orca://pair#…`). The code carries the desktop's static X25519 key, a LAN endpoint and, optionally, a relay invite.
  - The phone starts on the relay and upgrades to a direct connection when the desktop is reachable, using hysteresis and a cooldown.
  - The address can be edited later without pairing again.
  - The relay requires an Orca account. LAN-only pairing doesn't.
- **Crypto:**
  - tweetnacl. The phone brings an ephemeral X25519 key and the desktop brings its static one.
  - HKDF over a transcript that binds the transport (direct or relay) and the relay host.
  - Separate keys per direction, secretbox frames, and a 64-bit counter in the nonce.
  - The desktop key is a 0600 file, so there is no forward secrecy if it leaks. Swarn's v0.1 has the same gap (`docs/threat-model.md`).
- **Approvals from the phone** are weak outside structured sessions. Orca detects a prompt from terminal text or a hook, and answers it by typing into the PTY: a number to allow, Escape to deny (`mobile/src/session/mobile-native-chat-permission-send.ts`).
- **Push goes through Orca's gateway on Cloud Run with plaintext title and body** (up to 80 and 180 characters; `cloud/packages/push-contract/src/send-messages.ts`). Apple, Google and Orca can all read the notification text.
- **Onboarding offers a "Yolo permissions" switch.**

These last three points are where Swarn is different on purpose. Swarn signs structured approval objects on the phone, encrypts push content end to end, and asks before writes by default.

## 6. Worktrees

- Worktrees live in `~/orca/workspaces`. A pool prepares checkouts in advance so creating one feels instant.
- After the agent's first piece of work, the branch and the workspace title are renamed from what it did.
- **`orca.yaml`** holds setup and archive scripts, default tabs and shared directories (symlinked `node_modules`). Parsing is strict: a duplicate key rejects the file, and a bad field drops only that field.
  - The setup script comes from the new worktree's revision; the archive script and `.worktreeinclude` come from the main checkout.
  - It exports `CONDUCTOR_ROOT_PATH` as an alias on purpose, so Conductor setups keep working.
- **Ports are discovered, not allocated.** Orca scans listening ports, attributes each one to a worktree through the process tree, and proxies `<label>.orca.localhost` to it.
- **A failed archive script blocks removal** unless the user accepts it. Unmerged branches are kept and listed for review.

## 7. Engineering habits

- **Ratchets:**
  - Files are capped at 300 lines for .ts and 400 for .tsx, and the baseline can only shrink.
  - A "reliability gates" manifest promotes a test from soak to blocking only after 100 clean runs over 14 days.
  - A custom lint plugin bans patterns such as module mocking.
- **About 660 end-to-end test files.** The rules: assert on what's on screen, not on the store, and never steal focus.
- **Recorded protocol fixtures for every agent:**
  - ACP scenarios with golden journal and render outputs.
  - Claude frames captured through the SDK.
  - A scriptable fake Claude CLI that can ignore SIGTERM and fork grandchildren.

## 8. What Swarn takes

The PRD changes that follow from this note are in §2.4, §4.2, §4.3, §5.6 to §5.9 and §8.3, and each one points back here. In short:

- **Copy:**
  - Coalesced text snapshots in the log.
  - Write-ahead user messages with a delivery state.
  - Turn records with outcome and duration.
  - Bounded tool output.
  - A queue with steer.
  - Tool-run sentences and folded turns.
  - Approvals in the composer's place, with number keys.
  - Following the tail only until a reader's gesture.
  - Agent-written workspace titles.
  - Port attribution for the run panel.
- **Already have, keep:**
  - Explicit permission policy on every start.
  - Single-winner approval resolution (the daemon's one database writer).
  - Agents owned by a daemon that outlives the app.
- **Don't copy:**
  - Two chat pipelines (terminal scraping and structured).
  - The PTY keystroke approvals.
  - Plaintext push.
  - An account-gated relay.
  - The size.
