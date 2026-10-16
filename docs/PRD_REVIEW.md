# Technical PRD review: open questions

Review of `TECHNICAL_PRD.md` (15 Oct 2026), written 16 Oct 2026. The PRD revision of 16 Oct, after the competitor research, settled items 1, 10, 12, 13 and 19, and week 2 settled item 11; they are marked below. Everything else is still open. Items 1 to 6 change the schema or key storage, so settle them before the first commit. The rest can become tickets.

## Breaks the core scenario

- [x] **1. Mac sleep.** *Settled: PRD §5.5 step 9 holds an IOKit power assertion while any session is working or waiting.* An idle Mac sleeps, which suspends the daemon and its agents, so no push goes out while the user is away. Should the daemon hold an `IOPMAssertion` (PreventUserIdleSystemSleep) while any session is `working` or `needs_input`? What about lid-closed laptops on battery?
- [ ] **2. Daemon key on a locked Mac (§2.2).** `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` makes the key unavailable while the Mac is locked, which is the normal state when the user is away. These accessibility classes also only apply to the data-protection keychain, and that keychain needs an entitlement from a signed bundle, which a bare launchd binary doesn't have. Login keychain, or `AfterFirstUnlock`? Check in week 1.
- [ ] **3. Approve handling on iOS (§6.2 step 5).** The NSE runs once, before the notification is displayed. It doesn't receive action taps; the app does, launched in the background through `UNUserNotificationCenterDelegate`. The Approve action also needs `.authenticationRequired`, or the WhenUnlocked Enclave key fails on the lock screen. Rewrite the flow.
- [ ] **4. APNs 4 KB payload limit (§2.3).** After base64, only the 512 and 2048 padding buckets fit. Push payloads should drop `detail` and cap at 2048. State this in the envelope section.

## Security claims that don't hold

- [ ] **5. Gateway capability issuance (§7.2).** The Worker is stateless and has no stored P-256 key, so checking a signature made with a key supplied in the same request proves nothing. Anyone can sign a request for any token they know. §2.5 step 5 also says the daemon requests the capability, but the daemon has no device private key, and renewal at 25 days has the same problem. Options: App Attest, a device-signed request carried inside `pair_request`, or saying plainly that the capability is only a rate-limit token.
- [ ] **6. Replay of live frames (§2.3).** HPKE base mode plus a signature, with no counter, lets an attacker on the LAN replay `prompt`, `cancel`, `set_policy` or `create_workspace`. Only approvals carry a nonce. Add a per-peer monotonic counter inside the plaintext?
- [ ] **7. "LAN already wrapped in WireGuard" (§2.1).** True for the Tailscale candidates only. The `lan` and `mdns` candidates are plain WebSocket. Either reword the justification for dropping forward secrecy or accept the weaker claim explicitly.
- [ ] **8. "No button ever fails silently" (§2.7).** The `/healthz` probe runs when the notification arrives, but the user may tap minutes later after the daemon has gone unreachable. A failed post needs a local follow-up notification.
- [ ] **9. Stale lock-screen entries.** When the desktop approves first, the phone notification stays up. Removing it needs a background push, and iOS throttles those. Is a stale entry that answers "already resolved" acceptable?

## Spec inconsistencies

- [x] **10.** *Settled: the new mapping table in PRD §4.2 maps `permission_resolved` to `working`. Item 11 still decides what happens with several pending requests.* Originally: the ACP mapping table (§4.2) had no row for `permission_resolved`, so nothing returns a session to `working`.
- [x] **11.** *Settled in week 2: `next_status` takes the number of permission requests still open (PRD §4.2).* Originally: With parallel tool calls there can be several pending requests. `next_status(current, ev)` can't tell when the last one resolves. Does it need a pending count?
- [x] **12.** *Superseded: v0.1 no longer uses ACP (PRD §4.2). The same question now applies to Claude `result` subtypes and Codex turn statuses, and the contract transcripts (§9.2) pin it.* Originally: the ACP stop reasons `max_tokens`, `max_turn_requests` and `refusal` have no mapping.
- [x] **13.** *Superseded: Codex app-server has `acceptForSession`, which `allow_session` maps to; for Claude Code it becomes a session-scoped allow rule in the `control_response`.* Originally: `allow_session` is not an ACP permission kind. ACP agents send their own `options` with an `optionId` and a kind (`allow_once`, `allow_always`, `reject_once`, `reject_always`). Should the approval carry the agent's `optionId`?
- [ ] **14.** The SQL `session.status` includes `starting`, and the UDL `SessionStatus` enum doesn't. §4.2 mentions a PTY status `running` that appears in neither.
- [ ] **15.** Device envelopes are signed with the Enclave key, so the phone must be unlocked to send even `hello`, `subscribe` or `ack`. Should envelopes use a software key, with the Enclave key kept for approvals only?
- [ ] **16.** `event_writer` batches every 50 ms. Should `permission_request` flush immediately?

## Platform and dependency risks

- [ ] **17. mDNS on iOS.** `mdns-sd` sends raw multicast, which requires the `com.apple.developer.networking.multicast` entitlement. Apple grants it by manual request, and approval takes a while. Should discovery move to `NWBrowser` in Swift (only `NSBonjourServices` needed) and leave the Rust core?
- [ ] **18. License allowlist (§1.5, §8.1).** The release bar says every dependency is on the allowlist, but tokio, serde, ciborium, tungstenite and ulid aren't listed, and `gpui` pulls in Zed's `sum_tree`, `util` and `collections` transitively. Replace the hand-written table with `cargo-deny` and a license allowlist over the full graph?
- [x] **19. Node runtime.** *Settled: the drivers talk to `claude` and `codex app-server` directly, so v0.1 has no Node dependency (PRD §4.2).* The Claude Code and Codex ACP adapters are Node packages. How does the notarized DMG install them: bundle Node, or require it?

## Schedule

- [ ] **20. Week 6 load.** The week holds the iOS app, the NSE, the gateway, a real-device push test and the store submission, all for one person.
- [ ] **21. App Review.** An app that only works when paired with a Mac daemon is likely to be rejected under guideline 2.1 without a demo mode or reviewer setup notes. Plan a demo mode before week 6.
- [ ] **22. Week 1 load.** Core types, daemon skeleton, GPUI window, license audit and CLI timeout measurements all land in the same week.
