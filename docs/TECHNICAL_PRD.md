# Swarn: technical PRD

Companion to the product PRD. That document says what Swarn does and why; this one says how it is built, in enough detail that an implementer (human or agent) can start writing code without inventing decisions. Where the two disagree, the product PRD wins on scope and this one wins on mechanism.

Date: 15 Oct 2026, revised 16 Oct 2026. Owner: Sebastien, solo, working with coding agents.

The 16 Oct revision follows the competitor research in `docs/research/competitor-onboarding.md`. It replaces the ACP adapters with direct drivers for each CLI (§4.2), adds the first-run flow (§5.5), resolves open item 4, and moves the worktree and GitHub details closer to what shipping tools learned the hard way.

## 0. Scope of record for v0.1

The product PRD lists more than one person can build in eight weeks. Four cuts apply, and the rest of this document assumes them:

1. No native terminal in v0.1. The MVP loop runs on Claude Code and Codex only, each driven through its own structured interface (§4.2). The `alacritty_terminal` grid, the PTY tier and the generic ACP tier for other agents move to v0.2.
2. No tree-sitter in v0.1. Diffs render as monospace text with added/removed colouring. Syntax highlighting moves to v0.2.
3. No Android in v0.1. macOS desktop plus iOS. Android lands in v0.2 on the same Rust core.
4. No design system of our own in v0.1. The desktop copies Plexo's look (github.com/anmolkapil/plexo, MIT): its palette, Manrope and Roboto Mono, and its window chrome, ported to GPUI in `swarn-desktop/src/theme.rs`. Changed 16 Oct 2026; this replaces the earlier plan of `gpui-component` defaults with a stock Zed theme.

What that leaves: start Claude Code and Codex agents in git worktrees on a Mac, watch their status, get an encrypted push on an iPhone when one needs a human, approve or deny or reply from the notification, read the diff on the desktop, open a PR through `gh`.

Everything below marked v0.2+ is designed for but not built in the first eight weeks. The protocol and schema are designed once, with the relay in mind, because retrofitting a transport is expensive and designing for one is not.

## 1. Architecture

### 1.1 Processes

Four processes exist in v0.1, and only three of them are ours.

| Process | Language | Lifetime | Owns |
| --- | --- | --- | --- |
| `swarnd` (daemon) | Rust, tokio | launchd user agent, survives UI restarts | agent child processes, worktrees, event log, pairings, approval policy |
| Swarn.app (desktop) | Rust, GPUI | user session | rendering only; no agent state of its own |
| Swarn iOS | Swift + Rust core | OS-managed | pairing keys, local cache, notification extension |
| Push gateway | TypeScript, Cloudflare Worker | always on, stateless | signing APNs requests, nothing else |

The agent CLIs themselves are children of the daemon. Killing the desktop app never kills an agent. This is the single most important structural property and it is worth protecting in review: if the desktop ever spawns an agent directly, that is a bug.

### 1.2 Trust boundaries

Trusted: the daemon, the desktop app on the same machine, and a paired device's keys.

Untrusted: the push gateway, APNs, the relay (v0.2), Supabase (v0.5). They see ciphertext, device tokens, frame sizes, timing, and (gateway only) Live Activity status counts.

The desktop app talks to the daemon over a Unix domain socket with filesystem permissions as the only authentication (mode 0600, owned by the user). No crypto on that hop. A process running as the same user can already read the worktrees and the agent credentials, so encrypting that socket would buy nothing.

The phone is different. Even on the LAN it gets no implicit trust: every frame between daemon and device is sealed to the peer's key, so an attacker on the same Wi-Fi sees only lengths and timing.

### 1.3 Transport by release

| Release | Live channel | Offline delivery | Push |
| --- | --- | --- | --- |
| v0.1 | WebSocket, phone dials daemon directly on LAN or Tailscale | none; daemon asleep means no catch-up | gateway to APNs |
| v0.2 | WebSocket to self-hosted relay, both sides dial out | relay mailbox, 72 h, ciphertext | gateway to APNs and FCM |
| v0.5 | Supabase Realtime, user's own project | Postgres table of ciphertext | Edge Function to gateway |

A `Transport` trait in `swarn-core` has one implementation in v0.1 (`DirectWs`) and gains `RelayWs` in v0.2. The trait is written now because the mobile core and the daemon both consume it, and because writing it later means touching both.

```rust
#[async_trait]
pub trait Transport: Send + Sync {
    async fn connect(&mut self, peer: PeerId) -> Result<(), TransportError>;
    async fn send(&self, frame: SealedFrame) -> Result<(), TransportError>;
    fn incoming(&self) -> BoxStream<'static, Result<SealedFrame, TransportError>>;
    /// v0.2: replay from the mailbox. v0.1 returns Unsupported.
    async fn replay_after(&self, seq: u64) -> Result<Vec<SealedFrame>, TransportError>;
}
```

### 1.4 Repository layout

One Cargo workspace plus two platform directories.

```
swarn/
  Cargo.toml                  # workspace
  crates/
    swarn-core/               # types, session schema, HPKE envelopes, pairing, Transport
    swarn-daemon/             # tokio runtime, agent drivers, worktrees, SQLite, local API
    swarn-desktop/            # GPUI app, binary target `swarn`
    swarn-mobile/             # UniFFI wrapper over swarn-core + local cache
    swarn-bench/              # perf harness, simulated agents
    swarn-relay/              # v0.2
  ios/Swarn.xcodeproj         # SwiftUI app + notification service extension
  android/                    # v0.2
  gateway/                    # Cloudflare Worker, TypeScript
  docs/                       # this file, threat model, ADRs
  .github/workflows/
```

`swarn-core` has no tokio dependency and no I/O. It is pure types, serialisation, and crypto, so it compiles for aarch64-apple-ios and later for Android without pulling a runtime. The daemon and the mobile wrapper own all I/O. This is the boundary that makes "write the protocol once" real rather than aspirational.

### 1.5 Dependency allowlist

The Zed license audit is a release blocker, so the allowlist is explicit and small.

| Need | Crate | License |
| --- | --- | --- |
| UI framework | `gpui-pre` + `gpui-pre-platform` =0.3.9 (snapshot of zed@93d7456, the gpui that `gpui-component` builds on) | Apache-2.0 |
| Fonts | Manrope, Roboto Mono | OFL-1.1 |
| Widgets | `gpui-component` | Apache-2.0 |
| Terminal grid (v0.2) | `alacritty_terminal` | Apache-2.0 |
| HPKE | `hpke` | MIT/Apache-2.0 |
| P-256 verify | `p256` | MIT/Apache-2.0 |
| Ed25519 | `ed25519-dalek` | BSD-3 |
| SQLite | `rusqlite` (bundled) | MIT |
| Git | `git2` | MIT/Apache-2.0 |
| ACP (v0.2 generic tier) | `agent-client-protocol` | Apache-2.0 |
| mDNS | `mdns-sd` | MIT/Apache-2.0 |
| FFI | `uniffi` | MPL-2.0 |

Anything from the Zed tree other than `gpui`, `gpui-component` and `alacritty_terminal` is excluded until checked. `rope`, `sum_tree`, `ui` and the markdown crates are the ones we would want and the ones most likely to be GPL, which is one more reason v0.1 renders diffs as plain text.

## 2. Protocol and crypto

### 2.1 Two layers, and why

Live traffic and stored traffic have incompatible requirements. A live session wants forward secrecy and cheap framing. A push notification has to be decryptable by a notification extension with no session, no network, and a 30 second budget, possibly days after the daemon encrypted it.

So there are two layers, and only the first one is optional:

- Envelope layer (v0.1 and forever): one HPKE seal per message, no session state. Used for push blobs, relay mailbox frames, and in v0.1 for every live frame as well.
- Session layer (v0.2): Noise IK over the envelope-free path, rekeyed hourly, for live traffic once a relay sits in the middle.

v0.1 ships the envelope layer only. One crypto path in the mobile core, one set of test vectors, and the threat it drops (per-frame forward secrecy on a LAN already wrapped in WireGuard) is acceptable for eight weeks.

Ceiling: HPKE per frame costs an ephemeral X25519 keypair per message. At streaming rates (token deltas) this matters, so v0.1 batches deltas into at most 10 messages per second per session, and v0.2's Noise session removes the cost entirely.

### 2.2 Keys

| Key | Algorithm | Where it lives | Used for |
| --- | --- | --- | --- |
| Daemon static | X25519 | macOS Keychain, `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` | HPKE recipient, QR payload |
| Daemon signing | Ed25519 | same | signing frames and address updates |
| Device static | X25519 | iOS Keychain access group `$(TeamID).dev.swarn.shared`, `kSecAttrAccessibleAfterFirstUnlock` | HPKE recipient; the extension must open pushes while the screen is locked |
| Device signing | ECDSA P-256 | Secure Enclave, `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, no biometric ACL by default | signing approvals |

The asymmetry is deliberate and worth stating plainly, because it looks like a mistake otherwise. The Secure Enclave does P-256 and nothing else, so it cannot hold an X25519 key or do HPKE. Therefore the decryption key is a software key in the Keychain and the signing key is hardware-backed. The consequence: an attacker with code execution in the app sandbox can read queued notification text, but cannot forge an approval. An attacker with a locked, stolen phone can do neither.

The accessibility classes carry the product rule. `AfterFirstUnlock` on the X25519 key is what lets the extension decrypt a notification on the lock screen. `WhenUnlocked` on the Enclave key is what makes "the device must be unlocked to approve" true by construction rather than by a check we could forget.

If the user's approval policy demands biometrics for a tool, the Enclave key gets a second instance created with `.userPresence` and that instance signs those approvals. Two keys, one policy flag, no runtime prompt logic in the extension.

### 2.3 Envelope format

Every message on the wire is one `SealedFrame`, serialised with CBOR (`ciborium`) because it is self-describing enough to debug and compact enough not to care.

```
SealedFrame = {
  v:    u8,            # envelope version, 1
  to:   [u8; 32],      # recipient static X25519 public key, truncated SHA-256 in v2 if size matters
  enc:  bytes,         # HPKE encapsulated key, 32 bytes
  ct:   bytes,         # HPKE sealed ciphertext
  sig:  bytes,         # signature over (v || to || enc || ct)
  from: [u8; 32],      # sender signing public key, for key lookup
  pad:  bytes,         # zero padding to a size bucket
}
```

HPKE suite: `DHKEM(X25519, HKDF-SHA256)`, `HKDF-SHA256`, `ChaCha20Poly1305`. Mode: base. The `info` parameter is the ASCII string `swarn/v1/` followed by the frame purpose (`session`, `push`, `mailbox`), which domain-separates the three uses so a captured push blob cannot be replayed as a session frame.

AAD is the concatenation of sender signing key, recipient static key, and purpose. Signatures are Ed25519 when the daemon sends and ECDSA P-256 over SHA-256 when a device sends.

Padding buckets: 512, 2048, 8192, 32768, 262144 bytes. Anything larger is chunked by the session layer. The buckets exist so a relay cannot read a transcript by watching lengths; five buckets is a weak defence but it is almost free and the alternative is nothing.

The plaintext inside `ct` is a `SessionMessage`, JSON. JSON rather than a binary format because the whole thing is already inside an AEAD and being able to read a decrypted frame in a log during week 6 is worth more than the bytes. Ceiling: if transcript sync on cellular turns out to be slow, swap to CBOR behind the same serde types.

### 2.4 Session schema v1

Versioned independently of the envelope. The daemon advertises a supported range; the device picks the highest it also supports.

Device to daemon:

| Message | Fields | Notes |
| --- | --- | --- |
| `hello` | `schema_min`, `schema_max`, `device_id`, `app_version` | first frame after connect |
| `subscribe` | `scope` (`all` or session ids), `after_seq` | after_seq 0 means send a snapshot |
| `prompt` | `session_id`, `text`, `attachments[]` | follow-up into a running session |
| `approve` / `deny` | see 2.6 | signed separately from the envelope |
| `cancel` | `session_id` | sends the driver's interrupt, then SIGINT |
| `create_workspace` | `repo_id`, `base_ref`, `prompt`, `agent` | v0.2 from the phone; daemon supports it in v0.1 for the desktop |
| `ack` | `through_seq` | lets the relay drop mailbox frames in v0.2 |

Daemon to device:

| Message | Fields | Notes |
| --- | --- | --- |
| `hello_ack` | `schema`, `machine_id`, `machine_name`, `caps[]` | negotiated schema echoed back |
| `snapshot` | `workspaces[]`, `sessions[]`, `seq` | one frame, the whole board |
| `event` | `seq`, `session_id`, `kind`, `payload`, `ts` | the only streaming message |
| `gap` | `from`, `to` | v0.2 only, see 3.4 |
| `addresses` | `candidates[]`, `sig` | sent via push when the daemon moves networks |
| `error` | `code`, `message`, `retryable` | |

Event kinds, which are also the rows in the event log:

`session_started`, `status_changed`, `agent_message_delta`, `agent_message_end`, `thought_delta`, `tool_call`, `tool_call_update`, `plan`, `permission_request`, `permission_resolved`, `usage`, `diff_stat`, `session_ended`, `error`.

Clients must ignore unknown event kinds rather than erroring. That one rule is what makes adding an event kind a non-breaking change and keeps the N/N-1 window from filling up with trivia.

### 2.5 Pairing

The QR code is the whole trust transfer, so it carries everything the phone needs and nothing it should not keep.

```
swarn://pair?v=1
  &m=<machine_id>                       # 16 random bytes, base64url
  &n=<machine name, percent-encoded>
  &sk=<daemon static X25519 pubkey>     # 32 bytes, base64url
  &vk=<daemon Ed25519 verify key>
  &pw=<pairing window token>            # 16 bytes, valid 120 s, single use
  &c=<candidates>                       # see 2.7
  &g=<gateway base URL>
```

Steps:

1. The desktop asks the daemon to open a pairing window. The daemon generates `pw`, stores it with a 120 second expiry, and renders the QR.
2. The phone scans, generates its X25519 pair (Keychain) and its P-256 pair (Enclave) if it has none, and opens a WebSocket to the first candidate that answers.
3. The phone sends a `pair_request` sealed to the daemon's static key: device id, X25519 public key, P-256 public key, device name, APNs token, and `pw`.
4. The daemon verifies `pw`, checks that the P-256 key signs a challenge derived from `pw`, stores the pairing, and burns the token.
5. The daemon asks the gateway for a push capability for that APNs token (7.2) and returns `pair_ack` with its machine name, the schema range, and the capability expiry.
6. The desktop shows the new device in a list with a revoke button.

Pairwise keys per machine and device. Three machines and two phones means six scans. Vouching (a trusted device introduces a new machine) is designed for and deferred past v0.5.

Revocation deletes the pairing row, adds the device id to a deny list the relay will read in v0.2, and stops capability renewal. Frames already on the device stay readable; there is no remote wipe and the docs say so.

### 2.6 Approvals

The one place where getting the details wrong is a security bug rather than a bug.

A `permission_request` event carries:

```json
{
  "request_id": "01JA7...",        // ULID, unique per daemon
  "session_id": "...",
  "tool": "write_file",
  "summary": "Write src/main.rs (42 lines)",
  "detail": "...",                  // full tool input, shown in the app not the notification
  "risk": "write",                  // read | write | exec | network
  "nonce": "<16 random bytes>",
  "expires_at": "2026-10-15T14:03:00Z"
}
```

The device replies with:

```json
{
  "type": "approve",
  "request_id": "01JA7...",
  "nonce": "<echoed>",
  "decision": "allow_once",        // allow_once | allow_session | deny
  "device_id": "...",
  "signed_at": "...",
  "sig": "<P-256 over SHA-256 of the signing input>"
}
```

Signing input is the UTF-8 concatenation, with 0x1F separators, of: `swarn/approval/v1`, `request_id`, `nonce`, `decision`, `device_id`, `signed_at`. Separators matter; without them an attacker can shuffle field boundaries.

The daemon rejects an approval if any of these hold: unknown `request_id`, `nonce` mismatch, signature invalid for that device's stored P-256 key, `request_id` already resolved, `signed_at` more than 60 seconds in the future, or `expires_at` already passed. Every acceptance and rejection is appended to the event log as `permission_resolved` with the decision, device id, and reason on rejection.

Expiry is ours, not the CLI's. Neither CLI times a pending approval out: the Claude Agent SDK's `canUseTool` "can stay pending indefinitely", Claude Remote Control keeps prompts open until answered, and ACP specifies no timeout (sources in `docs/research/competitor-onboarding.md` §3.12–3.14). So `expires_at` is `default_expiry_secs` from the policy, 30 minutes unless changed, which keeps a notification from sitting on a lock screen for a day pretending to be live. Week 2 still confirms the "never times out" behaviour against the real CLIs.

When a request expires, the daemon answers the agent with an explicit deny and logs `permission_resolved` with reason `expired`, never as a user decision. Conductor shipped the bug where timeouts were "reported as user denials" (0.83.0); the event log has to tell the two apart.

A text reply that arrives while a permission is pending does not resolve or cancel the request. The daemon holds the text and delivers it after the request resolves. Happy lost pending requests this way (#1735).

An approval tapped after expiry is not sent. The extension detects it locally against `expires_at` and shows "Expired, open the app".

Policy lives in `~/.config/swarn/policy.toml`, read only by the daemon, never from a worktree. An agent that can write files in its own worktree therefore cannot widen its own permissions.

```toml
[approvals]
require_biometric_for = ["exec", "network"]
auto_deny = ["rm -rf /", "git push --force"]
default_expiry_secs = 1800
```

Policy changes require a signed `set_policy` message from the desktop (Unix socket, so implicitly local) or a paired device. Editing the file by hand works too; it is the user's machine.

### 2.7 Discovery and reachability

The QR candidate list, and every later `addresses` update, is an ordered array:

```json
[
  {"kind": "tailscale", "host": "mac-mini.tail1234.ts.net", "port": 8787},
  {"kind": "tailscale_ip", "host": "100.x.y.z", "port": 8787},
  {"kind": "lan", "host": "192.168.1.42", "port": 8787},
  {"kind": "mdns", "host": "swarn-macmini._swarn._tcp.local", "port": 8787}
]
```

The phone races all candidates with a 2 second budget, keeps the winner keyed by the current network's SSID or interface, and reorders the list so the winner is tried first next time. Tailscale first because it is the only one that works away from home.

The daemon re-publishes the list through a push when its addresses change, signed with its Ed25519 key so a hostile gateway cannot redirect a phone to an attacker. The phone verifies the signature against the key it stored at pairing and ignores anything that fails, which makes the gateway unable to do more than withhold updates.

mDNS registration uses `mdns-sd` under `_swarn._tcp`, instance name from the machine name. iOS needs `NSLocalNetworkUsageDescription` and the user prompt happens during pairing, not later when a connection silently fails.

The reachability probe in the notification extension is a plain `GET /healthz` against the remembered address with a 2 second timeout. If it answers, the notification shows Approve and Deny. If it does not, the notification shows the request text with Open app only. No button ever fails silently, which is the difference between "push works" and "push is broken" in a first-week bug report.

`/healthz` is the only unauthenticated endpoint on the daemon's listener. It returns `{"machine_id": "...", "schema": [1,1]}` and nothing else.

### 2.8 Version skew

The daemon supports schema N and N-1 and keeps N-1 for at least six months after N ships. `hello` carries the device's range; `hello_ack` carries the negotiated version. On no overlap the daemon returns `error` with code `schema_too_old` or `schema_too_new`, and the app renders "This machine needs an update" or "Update Swarn", never a spinner.

Hard rule for releases: a store build that understands schema N ships before any daemon release that requires N. The daemon auto-updates and the phone does not, so the daemon is always the one that has to be generous.

## 3. Data model

### 3.1 Daemon SQLite

One file, `~/Library/Application Support/swarn/state.db`, WAL mode, `synchronous=NORMAL`. Opened by the daemon only; the desktop never touches it.

```sql
CREATE TABLE machine (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  schema_min    INTEGER NOT NULL,
  schema_max    INTEGER NOT NULL,
  created_at    TEXT NOT NULL
);

CREATE TABLE repo (
  id            TEXT PRIMARY KEY,
  path          TEXT NOT NULL UNIQUE,
  default_branch TEXT NOT NULL,
  remote_url    TEXT,
  setup_script  TEXT,
  created_at    TEXT NOT NULL
);

CREATE TABLE workspace (
  id            TEXT PRIMARY KEY,
  repo_id       TEXT NOT NULL REFERENCES repo(id),
  name          TEXT NOT NULL,
  branch        TEXT NOT NULL,
  base_ref      TEXT NOT NULL,
  worktree_path TEXT NOT NULL,
  state         TEXT NOT NULL,     -- creating|ready|archived|failed
  created_at    TEXT NOT NULL,
  archived_at   TEXT
);

CREATE TABLE session (
  id            TEXT PRIMARY KEY,
  workspace_id  TEXT NOT NULL REFERENCES workspace(id),
  agent         TEXT NOT NULL,     -- claude-code|codex|custom
  tier          TEXT NOT NULL,     -- native|acp|pty (acp and pty from v0.2)
  status        TEXT NOT NULL,     -- starting|working|needs_input|done|failed|exited
  pid           INTEGER,
  acp_session   TEXT,              -- agent-side session or thread id for resume
  started_at    TEXT NOT NULL,
  ended_at      TEXT,
  last_change_at TEXT NOT NULL
);

CREATE TABLE event (
  seq           INTEGER PRIMARY KEY AUTOINCREMENT,  -- monotonic per daemon
  session_id    TEXT NOT NULL REFERENCES session(id),
  kind          TEXT NOT NULL,
  payload       TEXT NOT NULL,     -- JSON
  ts            TEXT NOT NULL
);
CREATE INDEX event_session_seq ON event(session_id, seq);

CREATE TABLE approval (
  request_id    TEXT PRIMARY KEY,
  session_id    TEXT NOT NULL REFERENCES session(id),
  tool          TEXT NOT NULL,
  risk          TEXT NOT NULL,
  nonce         BLOB NOT NULL,
  expires_at    TEXT NOT NULL,
  resolved_at   TEXT,
  decision      TEXT,
  device_id     TEXT,
  sig           BLOB
);

CREATE TABLE device (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  platform      TEXT NOT NULL,
  x25519_pub    BLOB NOT NULL,
  p256_pub      BLOB NOT NULL,
  push_token    TEXT,
  capability    TEXT,              -- gateway token
  capability_exp TEXT,
  paired_at     TEXT NOT NULL,
  revoked_at    TEXT
);

CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);
```

`event.seq` is a single global sequence per daemon rather than per session. One counter is simpler to reason about for resume ("everything after N"), and a client that only wants one session filters on `session_id` with the index above. Cross-machine ordering never uses `seq`; it uses `last_change_at` in UTC, as the product PRD says.

### 3.2 Event payloads

Payloads are JSON blobs whose shape is fixed per kind. Three that matter:

```json
// agent_message_delta
{"text": "...", "index": 17}

// tool_call
{"call_id": "...", "name": "write_file", "input": {...}, "status": "pending"}

// usage
{"input_tokens": 12043, "output_tokens": 882, "cost_usd": 0.14, "context_pct": 38}
```

`usage` is written only when the agent reports it. A missing meter renders as absent, never as zero. Fabricating a cost estimate from token counts and a price table we guessed is worse than showing nothing.

### 3.3 Retention

Active workspaces keep every event. Archived workspaces keep 30 days. A 2 GB default cap prunes oldest-archived-first. When only active logs remain and the cap is still exceeded, the daemon warns and keeps growing; it never truncates an active transcript, because a truncated transcript is a silent lie about what an agent did.

Pruning runs on daemon start and then hourly, in one transaction per workspace.

### 3.4 Gaps

v0.2 only. When a client asks for `after_seq = N` and the relay no longer has N, the relay sends `gap(from, to)`. The client marks that range missing, renders a visible marker in the transcript, and refetches the range from the daemon next time it is online. If the daemon has pruned it or the workspace is gone, the range becomes `unavailable`, a terminal state with its own marker. A hole in a transcript is always visible; that is the invariant.

### 3.5 Mobile cache

SQLite inside the app container, written by `swarn-mobile`, same event rows plus a `machine_id` column and a `pending_outbox` table for queued replies. Approvals never queue (they expire); plain text replies do.

## 4. Daemon

### 4.1 Task structure

Tokio multi-thread runtime. One task per concern, channels between them, no shared mutexes over anything bigger than a counter.

```
main
├── local_api        : UnixListener, JSON-RPC, one task per connection
├── device_listener  : TcpListener + WebSocket, one task per device connection
├── mdns             : registration and refresh
├── session_manager  : owns the session table, spawns per-session tasks
│   └── session/N    : agent driver, stdout/stderr pump, status derivation
├── event_writer     : single writer to SQLite, batched 50 ms, broadcasts to subscribers
├── push_sender      : queue to the gateway, retries with backoff
├── address_watcher  : network change notifications, pushes `addresses`
└── janitor          : retention pruning, capability renewal, expired approvals
```

One writer task for SQLite. Multiple writers to the same SQLite file in WAL mode work, but a single writer makes `seq` monotonic without a transaction dance, and it is the cheaper thing to reason about at 3am.

Ceiling: one global event writer caps throughput at a few thousand events per second, far above ten streaming agents. Shard by workspace if that ever shows up in a profile.

### 4.2 Agent drivers

v0.1 drives each CLI directly from Rust, through the interface its vendor publishes for other programs. There is no adapter process and no Node runtime in between.

| Agent | Process | Protocol |
| --- | --- | --- |
| Claude Code | the user's `claude`, unmodified: `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-prompt-tool stdio --permission-mode default` | newline-delimited JSON on stdio; permission prompts arrive as `control_request` messages of subtype `can_use_tool` and are answered with a `control_response`; `--resume <session_id>` continues a session |
| Codex | the user's `codex app-server` | JSON-RPC over stdio JSONL; `thread/start` and `turn/start` drive a session; approvals arrive as server requests such as `item/commandExecution/requestApproval`, answered with `accept`, `acceptForSession`, `decline` or `cancel` |

Why not the ACP adapters the first draft named. They have moved and changed shape: `zed-industries/codex-acp` is archived, and its replacement is a Node 22 package that itself wraps `codex app-server`. `@agentclientprotocol/claude-agent-acp` embeds its own pinned SDK and CLI, so it cannot drive the user's installed `claude`, and JetBrains documents that a Claude agent added through ACP cannot run on a Free, Pro or Max subscription. The tools that work well drive the vendor interfaces directly: Vibe Kanban and Sculptor use `claude -p` with stream-json, and Happy, Nimbalyst and Vibe Kanban use `codex app-server`. Driving them ourselves costs two small JSON protocols instead of one, removes Node from the install, and keeps Anthropic's auth policy simple: the end user signs in to the unmodified Claude Code binary themselves, which the policy explicitly allows. Evidence for all of this is in `docs/research/competitor-onboarding.md` §1, §3.12–3.14 and §4.2. ACP comes back in v0.2 as the generic tier for agents without their own interface (Gemini, Copilot and others).

Binaries are the user's own, found through the captured login-shell `PATH` (§5.5). Swarn does not bundle Claude Code: shipping it inside a product requires Anthropic's Commercial Terms. The daemon records each CLI's version per session and warns, without refusing, when a version has no recorded contract transcript (§9.2).

The permission mode is always passed explicitly. Without one, the Claude SDK's default "can be auto mode", and auto-approved calls never reach Swarn. Codex gets an approval policy per thread from Swarn's policy file rather than inheriting the user's `config.toml`; the old `approval_policy = "untrusted"` value is retired and can stop a client from starting.

Mapping onto Swarn events. Exact message and method names are pinned by the recorded transcripts in §9.2, not by this table:

| Swarn event | Claude Code (stream-json) | Codex (app-server) | Status effect |
| --- | --- | --- | --- |
| `session_started` | `system` message, subtype `init` | `thread/start` response | `working` |
| `agent_message_delta` | `stream_event` text delta | agent message delta notification | `working` |
| `thought_delta` | `stream_event` thinking delta | reasoning delta notification | `working` |
| `tool_call` | `tool_use` block in an `assistant` message | `item/started` for a command, file change or tool item | `working` |
| `tool_call_update` | `tool_result` block in a `user` message | `item/completed` | `working` |
| `plan` | `TodoWrite` tool call | plan update notification | `working` |
| `permission_request` | `control_request`, subtype `can_use_tool` | `item/*/requestApproval` server request | `needs_input` |
| `permission_resolved` | our `control_response` | our reply to the server request | `working` |
| `usage` | `result` message usage and cost | token usage notification | unchanged |
| `agent_message_end` | `result` message, subtype `success` | `turn/completed` | `done` |
| `session_ended` | process exit after an interrupt | `turn/interrupt` completed | `exited` |
| `error`, `session_ended` | non-zero exit or an error `result` | non-zero exit or error notification | `failed` |

Status is derived in exactly one place, a pure function over the last event and the current status, which means it is unit-testable without a child process:

```rust
pub fn next_status(current: Status, ev: &EventKind) -> Status
```

Permission requests are the interesting path. The agent blocks until the daemon answers, so the daemon writes the `permission_request` event, fires a push, and holds the reply until an approval arrives, the request expires, or the session is cancelled. Nothing about this is polled; it is a oneshot channel per request.

Two risks to watch. Claude's headless docs say `--bare`, which skips OAuth and the Keychain and needs an API key, "will become the default for -p in a future release"; if that lands, subscription users need a flag to opt out or the Agent SDK path, and the contract tests will catch the change. And agents spawned by a background process have failed to authenticate because they could not reach the macOS Keychain (codex-acp #310, claude-agent-acp #548), which is why week 2 starts with that test under launchd.

v0.2 adds the PTY tier. Status for those sessions comes only from agent hooks (Claude Code `Notification` and `Stop`, Codex `notify`), delivered by the hook shelling out to `swarn hook ...` which writes to the Unix socket. No prompt-pattern heuristics. A CLI with no hooks shows `running` or `exited` and nothing more, and the status board says so rather than guessing.

### 4.3 Worktrees

Creating a workspace has to be transactional in the sense that matters: a failure leaves something the user can delete, not a half-state the next operation trips over.

```
1. INSERT workspace (state='creating')
2. git worktree add -b <branch> <path> <base_ref>        # git2, falling back to the git CLI
3. copy gitignored files matching .worktreeinclude (default: .env*) from the repo root
4. UPDATE workspace SET state='ready'
5. run the repo setup script with cwd=<path>, 10 min timeout, output to the event log
```

Worktrees live outside the repo, under `~/Library/Application Support/swarn/worktrees/<repo>/<workspace>`. Nested worktrees confuse tooling that walks up the tree (Crystal #152), and Conductor moved away from them.

Step 3 copies files that git ignores but the app needs, mostly `.env` files. Almost every competitor ended up doing this after users asked repeatedly (Crystal #39, Claude Squad #260). `.worktreeinclude` uses gitignore syntax and is the same file Claude Code reads, so one file serves both.

A failure in steps 1 to 3 sets `state='failed'` with the error in the log and leaves the directory in place. A failing setup script does not: the workspace stays `ready`, the failure is an `error` event on the workspace, and the user can still send the first prompt, as Conductor does since 0.57.0. Repos also get an optional run script and archive script next to the setup script. Archiving runs `git worktree remove`, then `git branch -D` only if the branch is merged or the user confirms, then deletes `refs/swarn/<workspace>` (v0.4 checkpoints).

`git2` does not cover every worktree operation cleanly, so the daemon shells out to `git` for `worktree add` and `worktree remove` and uses `git2` for diffs and status. Mixing them is a known inelegance and cheaper than either alternative. Revisit `gitoxide` after v0.5.

Port allocation for setup and run scripts: the daemon gives each workspace a block of 10 free ports from a configurable range (default 31000-31999, so 100 workspaces), exports the first as `SWARN_PORT`, and records the block. Conductor and Emdash both settled on 10 because apps often need a second port. Dev servers are not reachable from the phone before v0.5.

### 4.4 Local API

JSON-RPC 2.0 over `~/Library/Application Support/swarn/daemon.sock`, mode 0600. Methods:

`daemon.info`, `repo.add`, `repo.list`, `workspace.create`, `workspace.list`, `workspace.archive`, `session.start`, `session.prompt`, `session.cancel`, `session.subscribe`, `approval.resolve`, `device.pair_window`, `device.list`, `device.revoke`, `policy.get`, `policy.set`, `diff.get`, `pr.create`, `hook.report`.

`session.subscribe` upgrades the connection to a server-push stream of the same `event` messages the phone gets, minus the envelope. The desktop uses this and nothing else for live state, which keeps one code path for "apply an event to the UI model" across both clients.

`pr.create` shells out to `gh pr create`. We are not reimplementing GitHub's API for v0.1; `gh` is already installed on the target user's machine and already holds their token. The daemon checks `gh --version` and `gh auth status` during onboarding (§5.5) and again before each PR, and surfaces a specific error ("GitHub CLI not signed in, run gh auth login") rather than a failed subprocess. It also checks that git runs at all, because an unaccepted Xcode license breaks git and `gh` silently, which Conductor hit (0.36.7). `GH_TOKEN` in the daemon's environment is accepted as a fallback, as Conductor added in 0.25.5, because detecting `gh` auth was its most fragile onboarding step.

## 5. Desktop

### 5.1 Structure

GPUI with a pinned Zed tag. The app is a view tree over one observable model, and the model is a projection of daemon events.

```
swarn-desktop/src/
  main.rs            # app setup, window, keymap
  model/
    board.rs         # workspaces, sessions, statuses; applies events
    session.rs       # transcript rope, tool calls, pending approvals
    diff.rs          # parsed unified diff
  views/
    shell.rs         # sidebar + main pane split
    sidebar.rs       # workspace list with status dots
    transcript.rs    # virtualized list of rendered blocks
    approval.rs      # inline permission card
    diff_view.rs     # plain monospace diff
    palette.rs       # command palette
    pairing.rs       # QR sheet, device list
  theme.rs           # one Zed theme, loaded as-is
```

### 5.2 Rendering rules that keep the frame budget

The success metric is p99 under 8.3 ms on macOS while ten agents stream. Three rules get us there, and all three are structural rather than optimisations to add later:

1. Parsing happens off the main thread. The daemon connection task deserialises events, builds render-ready blocks, and sends them to the UI as diffs against the model. The UI thread applies and lays out; it never parses markdown.
2. Transcripts are virtualized with `uniform_list`, so only visible rows are laid out. A 50,000 line transcript costs the same as a 50 line one.
3. Streaming deltas coalesce. The model buffers `agent_message_delta` for 16 ms and appends once, so a chatty agent cannot force more than one relayout per frame.

Markdown rendering in v0.1 is deliberately shallow: paragraphs, code fences, lists, inline code, links. No tables, no nested blockquote styling, no syntax highlighting inside fences. It is a custom streaming element because incremental parsing of a growing string is the whole problem and no crate on the allowlist does it.

### 5.3 Diff view

`git2` produces the diff; we render it as a unified list of lines with three colours and a file header, plus mark-as-viewed state per file held in the model. Side by side, inline comments, and syntax highlighting are v0.2. The MVP question a diff view has to answer is "did the agent do something reasonable", and plain text answers it.

### 5.4 Keyboard

Every action is a GPUI action with a default binding, declared in one `keymap.json` shipped with the app and overridable from `~/.config/swarn/keymap.json`. The command palette is fuzzy search over the action registry, so a new action appears in the palette without being registered twice.

### 5.5 First run

The first launch is a short checklist, one screen per step. Every step can be re-run later from settings, and only step 1 is required. The order follows `docs/research/competitor-onboarding.md` §4.1, where each step is tied to the evidence behind it.

1. Register the daemon. `swarnd` ships inside the app bundle, and the app registers it as a per-user launch agent through `SMAppService`, so the user sees one Login Items approval and never opens a terminal. This answers the old open item 4 with a single code path. `swarnd install` stays for headless machines. Of the competitors, only Omnara registers a per-user launchd service cleanly; Conductor has no helper and loses local sessions when it quits.
2. Capture the environment. A launch agent gets a minimal `PATH` and no login shell, so the daemon runs the user's login shell once (`$SHELL -l -i -c env`, 5 second timeout), stores the result, and shows which tools it found. Superset broke when agent CLIs were only on the `.zshrc` `PATH` (#7398).
3. Check git and GitHub: git runs (the Xcode license check from §4.4), `gh` is installed and signed in, or `GH_TOKEN` is set. GitHub is skippable until the first PR.
4. Find and sign in to each agent, one card per agent, each skippable. The checks run inside `swarnd`, not the app, because the app's environment is not the one agents will run in: `claude auth status` and `codex login status`. To sign in, Swarn opens a terminal running `claude /login` or `codex login` and polls the status. Swarn never shows its own Claude login and never reads the token, per Anthropic's policy. A set `ANTHROPIC_API_KEY` gets a warning, because it silently overrides the subscription.
5. Add a repository: pick a local folder (`repo.add`). Cloning comes later.
6. Configure the repository: setup, run and archive scripts, and the `.worktreeinclude` patterns, prefilled with `.env*`.
7. Choose the approval policy. Three choices, defaulting to "ask me before writes and commands". The result goes to `~/.config/swarn/policy.toml`. Most competitors skip permissions by default; asking a human first is the point of Swarn, so it is the default here.
8. Ask for macOS notification permission, and explain the Files and Folders prompts that will name Swarn when an agent touches Desktop or Downloads.
9. Offer keep-awake. While any session is `working` or `needs_input`, the daemon holds an IOKit power assertion against idle sleep (the fix for review item 1). The screen says plainly that a closed lid on battery still sleeps, and that the phone gets nothing while the Mac is asleep (§1.3).
10. Pair a phone, optional. Before the QR appears, the Mac side says whether it found Tailscale or will be LAN-only, since v0.1 has no relay. The phone asks for camera, Local Network and notification permission during pairing, in that order (§2.5, §2.7).
11. Send a test approval end to end: a fake permission request from `swarnd`, through the push gateway, approved on the phone, back to the daemon. The top complaints about competitors' phone apps are pushes that never arrive and pairing that fails silently, so onboarding proves the path works before the user depends on it.
12. Start the first workspace from a prompt box.

## 6. Mobile

### 6.1 The UniFFI surface

The Swift app contains no protocol logic. `swarn-mobile` exposes a small object graph and the app renders it. Keeping this surface small is what makes Android in v0.2 a UI exercise instead of a second implementation.

```udl
namespace swarn {};

dictionary MachineInfo { string id; string name; boolean reachable; };
dictionary InboxItem {
  string session_id; string machine_id; string workspace_name;
  SessionStatus status; string summary; i64 changed_at_ms;
  PermissionRequest? pending;
};
enum SessionStatus { "Working", "NeedsInput", "Done", "Failed", "Exited" };

interface Core {
  constructor(string db_path, string keychain_group);

  // pairing
  PairResult pair([ByRef] string qr_url);
  sequence<MachineInfo> machines();
  void unpair(string machine_id);

  // connection
  void connect();                       // races candidates, reconnects with backoff
  void disconnect();
  ConnState conn_state();

  // reading
  sequence<InboxItem> inbox();
  sequence<Block> transcript(string session_id, u32 limit, u64 before_seq);

  // writing
  [Throws=CoreError] void send_prompt(string session_id, string text);
  [Throws=CoreError] void resolve(string request_id, Decision d);

  // notification extension entry point, usable with no live connection
  [Throws=CoreError] Notification open_push(bytes payload);
  [Throws=CoreError] bytes sign_approval(string request_id, bytes nonce, Decision d);

  void set_listener(CoreListener l);
};

callback interface CoreListener {
  void on_inbox_changed();
  void on_session_event(string session_id);
  void on_conn_state(ConnState s);
};
```

`open_push` and `sign_approval` are callable from the notification service extension, which means `swarn-mobile` must work with no network, no listener, and a cold start under 30 ms. That constraint drives the rest: no tokio in the extension path, Keychain access through a group both targets share, and the SQLite cache opened read-only there.

### 6.2 iOS targets

Three targets in one project: the app, `SwarnNotificationService` (the NSE), and a shared framework wrapping the Rust static library. App group and Keychain access group are shared across all three.

Notification flow:

1. APNs delivers `mutable-content: 1` with the sealed blob in `payload`.
2. The NSE calls `open_push`, which HPKE-opens the envelope with the Keychain X25519 key and returns title, body, and request metadata.
3. For a `permission_request`, the NSE probes `/healthz` on the remembered address, 2 second timeout.
4. Reachable: attach the `SWARN_APPROVE` category so Approve and Deny appear. Unreachable or expired: attach a category with Open app only.
5. On Approve, the extension calls `sign_approval` (Enclave key, so the device must be unlocked), posts it to the daemon, and updates the notification.

The NSE budget is roughly 30 seconds, and the probe eats 2 of them. Everything else has to be local and fast, which is why the decryption key is a software Keychain key rather than something that needs an unlock.

### 6.3 Screens in v0.1

Four. Inbox, session, pairing (camera plus machine list), settings. No diff viewer, no Live Activity, no widget, no workspace creation from the phone. The design is `gpui-component`-equivalent plainness: system fonts, system colours, no custom components.

Reconnect behaviour: on foreground and on network change, `connect()` races candidates, then `subscribe(after_seq = last known)`. A failed race shows the Tailscale banner rather than a spinner.

### 6.4 Android, v0.2

Same `Core` through UniFFI's Kotlin bindings, `FirebaseMessagingService` in place of the NSE, StrongBox when available and TEE-backed Keystore otherwise, and an ongoing notification instead of a Live Activity. Before Android ships, check the Play developer account type: a personal account created after November 2023 must run closed testing with 12 testers for 14 days before production, which is a recruiting problem that cannot be solved in the last week of a sprint.

## 7. Push gateway

### 7.1 Shape

A Cloudflare Worker, no database, no accounts, no stored messages. It holds the APNs key for the App Store app and signs requests with it. That is the only thing it can do that a self-hoster cannot, which is why it exists at all.

```
POST /v1/capability   -> issue a capability for (push_token, pairing_id)
POST /v1/push         -> forward a sealed envelope to APNs
POST /v1/activity     -> Live Activity start/update (v0.3)
GET  /healthz
```

### 7.2 Capabilities

Self-verifying, so the Worker needs no storage:

```
capability = base64url(payload) "." base64url(ed25519_sig(payload))
payload    = {"th": sha256(push_token)[0..16], "pid": pairing_id, "exp": unix_secs}
```

Issued only for a request signed by the device's P-256 key over `(push_token, pairing_id, timestamp)`, so possession of someone else's token is not enough to get a capability for it. Valid 30 days, renewed by the daemon's janitor task at 25 days. Revoking a pairing stops renewal and the daemon drops the token immediately.

`/v1/push` requires a valid capability whose `th` matches the hash of the token in the request. Rejections are 403 and rate-limited by Cloudflare's rate limiting binding, keyed on `th`, at 60 pushes per minute per device. Knowing a device token is not enough to wake a phone; that was the hole in the first draft of this design.

Idempotency: the daemon sets `apns-collapse-id` to the `request_id` for permission requests, so a retry replaces rather than duplicates a lock screen entry.

### 7.3 What it can see

Device tokens, ciphertext sizes, timing, and Live Activity status counts in cleartext from v0.3 (iOS applies those updates without passing through the extension, so they carry counts and generic labels only, never prompts, code, or file names). This is written down in the threat model and in the privacy section of the store listing, because "end to end encrypted" with an asterisk is worse than a precise claim.

If the gateway is down, iPhones lose push. Live sessions, in-app approvals, and (from v0.2) the mailbox keep working.

## 8. Build plan

Eight weeks, one builder, agents doing most of the typing. The order is chosen so that the thing most likely to kill the project (GPUI being unworkable, or the Zed crates being GPL) is tested in week 1, and so that the App Store queue starts before the last week.

| Week | Deliverable | Done when |
| --- | --- | --- |
| 1 | `swarn-core` types and session schema; daemon skeleton with SQLite and the local API; GPUI hello-window; Zed crate license audit (done 16 Oct; the CLI timeout measurement moved to the week 2 spike) | A GPUI window renders a list from the daemon over the Unix socket; audit written to `docs/licenses.md` |
| 2 | Spike first: `claude auth status` and `codex login status` from `swarnd` running under launchd, plus login-shell environment capture, and confirm neither CLI times out a pending approval. Then the worktree lifecycle with `.worktreeinclude`; the Claude Code stream-json driver; status derivation; event log and subscribe stream | Spike results in `docs/cli-behaviour.md`; `swarn workspace create` then `session start` produces a live transcript in the terminal client |
| 3 | GPUI sidebar, transcript view with virtualization and streaming markdown, inline approval card | Approving from the desktop unblocks a real Claude Code run |
| 4 | Codex app-server driver; diff view; `gh` PR flow; OS notifications; command palette; first-run flow steps 1 to 9 (§5.5) with `SMAppService` registration and keep-awake; perf harness and first numbers | A fresh macOS user account goes from first launch to a running agent without opening a terminal except to sign in; `swarn-bench` reports RSS and p99 frame time |
| 5 | HPKE envelopes; pairing flow and QR; device listener; discovery candidates and mDNS; `swarn-mobile` UniFFI surface | An integration test pairs a simulated device and streams events end to end |
| 6 | iOS app: pairing, inbox, session view, reply; NSE with decrypt, probe, approve; push gateway Worker deployed; first-run steps 10 and 11 (§5.5) | Push to approval works on a real iPhone, and the onboarding test approval passes; **first App Store submission by end of week** |
| 7 | Reachability edge cases; reconnect and backoff; retention and janitor; threat model document; error states everywhere | A week of eating our own cooking with no sharp edges found twice |
| 8 | Dogfooding, bug fixes, release engineering: notarized DMG, auto-update, README, install script | Release bar met (below) |

From week 4, Swarn builds Swarn. The fallback is plain CLI agents in a terminal, because a dogfood build that breaks must not stop the work.

If week 6 slips, the cut is iOS polish, not the submission date: submit whatever passes review and fix forward. If the store slips, v0.1 ships as a desktop release plus TestFlight, with the store release following.

### 8.1 Release bar for v0.1

- Ten simulated agents streaming: UI plus daemon RSS under 250 MB, p99 frame under 8.3 ms on the dev Mac, cold start under 400 ms.
- Push to approval works end to end on a physical iPhone, including the locked-screen path.
- One week of daily real use with no open P0.
- Zed crate license audit complete, with every dependency on the allowlist.
- App Store approval received, or the documented desktop-plus-TestFlight fallback.

### 8.2 Accounts and costs, owned by Sebastien

Apple Developer ($99/yr, open it in week 1 because verification takes days), Google Play ($25, before v0.2), domain, Cloudflare account holding the APNs key, and the dev Mac that doubles as the perf runner.

## 9. Testing

The rule for a solo build with agents writing code: every non-trivial behaviour leaves behind one runnable check that fails if the behaviour breaks. Not a suite per function. One check that matters, in CI, so an agent can verify its own change without asking.

### 9.1 Unit and property tests

| Area | Check |
| --- | --- |
| Status derivation | Table test over `next_status` for every event kind and current status |
| Approval verification | Rejects wrong nonce, wrong signature, replayed request id, expired request, future timestamp. One test per rejection reason |
| HPKE envelopes | Round trip, plus fixed test vectors in `crates/swarn-core/tests/vectors.json` so a crate upgrade that changes the suite fails loudly |
| Signing input | Golden string test, so nobody reorders the fields or drops the separators |
| Padding buckets | Property test: sealed size is always one of the five buckets |
| Session schema | Round trip every message type; an unknown event kind deserialises and is ignored |
| Retention | Prune with a seeded database leaves active logs untouched |
| Candidate racing | Simulated candidates with injected latency pick the fastest and remember it |

### 9.2 Integration tests

- `tests/pair_and_stream.rs`: spawn a daemon with a temp database, pair a simulated device, start a scripted fake agent, assert the device receives the events in order and can approve.
- `tests/worktree_failure.rs`: a setup script that exits 1 leaves `state='failed'` and a removable directory.
- `tests/agent_contract/`: recorded stream-json and app-server transcripts per CLI version, replayed against the drivers. Pinned per version, because the whole point is to notice when a CLI changes shape. Re-record deliberately, never automatically. Claude Code shipped five adapter-level releases in one week in October 2026, so this suite earns its keep.

The fake agent is a small binary in `swarn-bench` that reads a script file and replays Claude Code stream-json messages on a timer, permission requests included. It is also what the perf harness and the onboarding test approval (§5.5 step 11) use, so there is one fake agent, not three.

### 9.3 Performance

`swarn-bench` spawns N fake agents at a configurable token rate, drives the real daemon and a headless-ish desktop, and reports RSS, p99 frame time, cold start, and workspace switch time as JSON.

CI runs it nightly on the idle dev Mac, ten runs per scenario, failing when the median breaches a budget or the spread exceeds 10 percent. There is no per-PR perf gate in v0.1: the only runner is also the machine being typed on, so a per-PR gate would flap and get ignored, which is worse than not having one. Pull requests run correctness tests only. When there is spare hardware, the smoke budget comes back with a 15 percent threshold against the rolling median of the last 20 main-branch runs.

### 9.4 iOS

XCTest for the inbox and session view models, and one UI test for the pairing camera flow with a stubbed QR payload. The NSE path gets a unit test against `open_push` with a fixture envelope, because an NSE bug shows up as a notification that silently says nothing.

### 9.5 CI matrix

macOS arm64 for the workspace and the iOS build. No macOS x86 (Intel is "by demand" now). Linux from v0.2. Relay install test on a fresh Ubuntu VM from v0.2. Note that iOS and macOS runner minutes cost money on a project with no revenue, which argues for keeping the matrix at arm64 and running the heavy jobs on the dev Mac as a self-hosted runner.

## 10. Open items

These are genuinely unresolved, not deferred:

1. Trademark search for "Swarn", plus `crates.io`, the GitHub org, the domain, and both store listings. Every `get.swarn.dev` and `ghcr.io/swarn` reference in these documents is a placeholder until that clears. The name also reads as a typo of "swarm", which is worth ten minutes of thought before the repo is public.
2. Whether `gh` stays the PR mechanism past v0.2 or we move to the GitHub API with a token in the Keychain. `gh` is right for v0.1 and wrong for a daemon that runs headless on a VPS.
3. Launch plan. Nothing in this document depends on it, and it decides whether the 5,000 weekly active users goal is a measurement or a wish.
4. Resolved 16 Oct 2026: the app registers the daemon through `SMAppService` on first run (§5.5 step 1).
5. Written guidance from Anthropic on an open-source app driving the user's own `claude -p` on a Free, Pro or Max subscription. The policy text allows users to sign in to the unmodified binary themselves and forbids apps from offering Claude.ai login or handling tokens; Swarn does neither, but ask before launch rather than infer. The same question exists for Codex: app-server auth is permitted for local and open-source apps and not for commercial or hosted ones, which matters if Swarn ever charges.
6. Differentiation. Claude Code Remote Control and the ChatGPT desktop app already run worktree sessions with phone approvals through their vendors' relays, and cmux has encrypted its pushes since 5 Oct 2026. What no one offers is a phone that talks to the Mac directly, with no vendor relay, and a human-approval default. That is a product PRD question, but it should shape what v0.2's relay promises.

