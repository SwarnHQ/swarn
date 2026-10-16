# Threat model

What Swarn v0.1 protects, from whom, and where it stops. It describes the code as it is, not the PRD as written; where the two differ, this file says so. If you find a gap that isn't listed under "Known gaps", that's a security bug. See `SECURITY.md` for how to report it.

## What's worth stealing

- **The power to approve.** An approval lets an agent write files or run a command as you. Forging one is the worst thing an attacker can do through Swarn.
- **Transcripts and diffs.** Prompts, agent output and code from your repositories. They live in the daemon's event log and pass over the wire to paired phones.
- **Pairing keys.** The daemon's identity and each phone's keys. With them, someone can read traffic or pose as a device.

Agent credentials (Claude and Codex logins) are not Swarn's to protect. The agents read them from your home directory. Swarn never sees them.

## Trust boundaries

Trusted:

- the daemon, `swarnd`
- the desktop app on the same Mac
- a paired device, for as long as it stays paired

Untrusted:

- the network, including your own Wi-Fi
- anything that connects to the device port without a pairing
- the push gateway and APNs, once they exist (week 6 hasn't shipped)

The desktop talks to the daemon over `daemon.sock`, a Unix socket with mode 0600 inside a 0700 directory (`~/Library/Application Support/swarn`). File permissions are the only check. No crypto runs on that hop. Anything running as your user can already read the worktrees and the agents' credentials, so encrypting that socket would buy nothing.

## Who might attack, and what they get

### Someone on the same network

Every frame between the daemon and a phone is one HPKE seal (X25519, HKDF-SHA256, ChaCha20-Poly1305), signed by the sender and padded to one of five sizes (512 B to 256 KiB). An observer sees which phone talks to which Mac, when, and roughly how much. They don't see content, and they can't inject frames: the daemon drops anything that fails to open or verify, and gives strangers no reply.

A recorded frame replayed later is dropped. Each plaintext carries a counter, and the daemon stores the last counter it accepted per device in the database, so a daemon restart doesn't reopen the window.

mDNS announces `_swarn._tcp` with the machine name to everyone on the LAN. `GET /healthz` is the one endpoint that needs no key. It returns the machine id and the schema range and nothing else.

### Someone who connects to port 8787 without pairing

They can open a WebSocket and send frames. Frames that don't decrypt to the daemon's key are ignored. A frame over 300 KiB closes the connection, and so does 30 seconds without pairing or showing a known device key. The daemon binds `0.0.0.0`, so this applies on every interface the Mac has, Tailscale included.

To pair, they need the token from a currently open QR code.

### Someone who sees the QR code

The QR holds the daemon's public keys, its addresses and a pairing token. The token is 16 random bytes, valid for 120 seconds, and burned on first use. Whoever scans it first inside that window becomes a paired device. They also have to sign the pairing request with the P-256 key they're registering, which proves they hold it.

Mitigations: the window only opens when you click Pair a phone, the sheet closes as soon as a device joins, and the desktop shows "Paired <name>" with the device in the list under Setup, where Revoke is one click. If a phone you don't recognise shows up, revoke it.

### A lost or stolen phone

The phone's design keeps the signing key in the Secure Enclave, readable only while the phone is unlocked (PRD §2.2). The iOS app isn't built yet, so this is the plan rather than tested code. Until you revoke the phone, someone holding it unlocked can approve anything a paired device can.

Revoking deletes the pairing and drops the phone's open connection at once. Frames already on the phone stay readable there. There is no remote wipe.

### A forged or replayed approval

A device approval must name a pending request, echo its random nonce, carry a valid P-256 signature from that device's stored key over a domain-separated input, and arrive before the request expires and no more than 60 seconds ahead of the daemon's clock. Anything else is refused, and the refusal goes into the event log with the reason. A request that times out is answered with a deny and logged as `expired`, never as a decision you made.

Approvals from the desktop go over the local socket and aren't signed. They're logged with the device id `local`.

### The agent itself

Prompt injection is out of scope for Swarn to prevent, but it shapes the boundaries. The approval policy lives in `~/.config/swarn/policy.toml`, outside any worktree. Only the daemon reads it, so editing files in its own worktree can't widen an agent's permissions.

That holds only as long as the approvals hold. Agents run as your user. Once you allow a shell command, it can do anything you can, including editing the policy file, reading other repositories, or pushing code. In `allow_edits` mode, Claude Code edits files without asking and Codex runs with its `workspace-write` sandbox. Commands still need an approval in both. Read the command before you approve it.

### Other software on your Mac

A process running as your user can read the event log and the daemon's keys, talk to the socket, and act as the desktop app. Swarn doesn't defend against malware that already runs as you. Other user accounts on the same Mac are kept out by file permissions.

## Known gaps

- **Daemon keys are a file, not the Keychain.** `identity.json` (mode 0600) holds the daemon's X25519 and Ed25519 private keys. The PRD calls for the Keychain. That waits for a signed build, because an ad hoc signed build would hit a Keychain prompt on every rebuild.
- **No forward secrecy per frame.** Frames are sealed to long-lived keys. Anyone who records traffic and later steals the daemon's X25519 key can read what was sent to the daemon. The Noise session layer planned for v0.2 fixes this for live traffic.
- **The event log is plaintext on disk.** `state.db` keeps transcripts in the clear, protected only by file permissions and FileVault. Archived workspaces lose their events after 30 days, and active ones keep everything.
- **Some policy fields don't exist yet.** v0.1 ignores the PRD's `require_biometric_for` and `auto_deny`. Saving the policy from the app rewrites the file without them.
- **No push yet.** The gateway in PRD §7 isn't built. When it ships, it will see device tokens, frame sizes and timing, and nothing in the clear beyond that.
- **The machine name is public on the LAN.** That comes from mDNS. Turning it off (`SWARN_NO_MDNS=1`) means phones on the LAN need the IP candidates in the QR instead.
