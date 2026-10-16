# How the agent CLIs behave when Swarn drives them

Week 2 spike from the technical PRD (§8). Measured on 16 Oct 2026, on macOS 27.0 (Apple silicon),
with Claude Code 2.1.300 and Codex CLI 0.147.0. Re-run this when either CLI changes major behaviour;
the contract transcript in `crates/swarn-daemon/tests/fixtures/` is what CI watches day to day.

## 1. Signing in from a launchd agent

The question: when `swarnd` runs as a per-user launch agent, can the agents it starts reach the
user's login? Competitors hit "Authentication required" here because the child could not reach the
Keychain (research doc §3.14).

How it was measured: a throwaway launch agent, loaded with `launchctl bootstrap gui/$UID` from a
plist in `/tmp` and booted out right after, ran both status commands by absolute path.

| Check | Result |
| --- | --- |
| `claude auth status` | `loggedIn: true`, `authMethod: claude.ai`, subscription detected. Exit 0. |
| `codex login status` | "Logged in using ChatGPT". Exit 0. |
| `PATH` launchd gives the job | `/usr/bin:/bin:/usr/sbin:/sbin` |
| Found on that `PATH` | `git` only. `claude`, `codex`, `node` and `gh` are all missing. |

So the Keychain is reachable from the `gui/<uid>` domain, which is the domain `SMAppService`
launch agents run in. A daemon started in another domain (over SSH, say) was not tested and may not
see the Keychain. The real problem is `PATH`, which leads to the next measurement.

## 2. Capturing the login shell's environment

`$SHELL -l -i -c 'printf "\n__SWARN_ENV__\n"; /usr/bin/env -0'`, started from an environment as bare
as launchd's (`env -i` with only `HOME`, `SHELL`, `USER` and the launchd `PATH`):

- Took 0.6 s with a real zsh setup (oh-my-zsh style, nvm, Homebrew).
- Nothing reached stdout before the marker, but the parser skips anything that does.
- Returned 25 variables and a 30-entry `PATH` on which `claude`, `codex`, `node`, `gh` and `git` all
  resolve.

`swarnd` now does this once at startup with a 5 second budget and falls back to its own environment
if the shell fails (`crates/swarn-daemon/src/env.rs`). Every agent is started with that environment.

## 3. Driving Claude Code over stream-json

### Flags that work

```
claude -p --input-format stream-json --output-format stream-json --verbose \
  --include-partial-messages --permission-mode manual --permission-prompt-tool stdio
```

Two corrections to the first version of PRD §4.2:

- There is no `default` permission mode any more. The choices are `acceptEdits`, `auto`,
  `bypassPermissions`, `manual`, `dontAsk` and `plan`. `manual` is the one that asks.
- `--permission-prompt-tool stdio` is required. Without it, `manual` mode denies the tool call on
  its own (a `system` message of subtype `permission_denied`), and the agent gives up. Sending the
  SDK's `initialize` control request does not change that. With the flag, `initialize` is not
  needed.

Version 2.1.300 also has `--permission-prompts host|none`. `host` is the default and is what
routes prompts to us, so Swarn doesn't pass it.

### What a permission prompt looks like

Claude writes a control request on stdout and waits:

```json
{"type":"control_request","request_id":"3545d1d7-…","request":{"subtype":"can_use_tool",
 "tool_name":"Write","display_name":"Write","input":{"file_path":"…/hello.txt","content":"hi"},
 "description":"hello.txt","permission_suggestions":[{"type":"setMode","mode":"acceptEdits",
 "destination":"session"}],"tool_use_id":"toolu_…"}}
```

Swarn answers on stdin:

```json
{"type":"control_response","response":{"subtype":"success","request_id":"3545d1d7-…",
 "response":{"behavior":"allow","updatedInput":{…}}}}
```

`{"behavior":"deny","message":"…"}` refuses. For "allow for this session", Swarn sends the
`permission_suggestions` back as `updatedPermissions`. In the run above that meant switching the
session to `acceptEdits`.

### Other things worth knowing

- **Cancelling:** a `control_request` of subtype `interrupt` stops the turn. Claude then ends it
  with a `result` of subtype `error_during_execution`, and the process stays alive for the next
  prompt. Swarn records that as the end of the turn with `stop_reason: "cancelled"`, not as a
  failure.
- **Follow-ups during a turn:** a user message sent while a turn is running gets folded into that
  turn. Claude does not start a second one.
- **Repeated `init`:** Claude sends `system`/`init` again on later turns, so Swarn only records the
  first one as `session_started`.
- **Cumulative cost:** `total_cost_usd` in each `result` is the total for the whole process, not
  for the turn.
- **The user's own config comes along.** The unmodified CLI loads the user's hooks, plugins, skills
  and CLAUDE.md. On this machine that added about 46k tokens of context to every session, so a
  one-line prompt cost $0.38 on the default model and $0.01 on Haiku. That's the price of running
  the real binary under the user's login, and it's worth saying in onboarding.
- **Personal data in raw transcripts:** `init`, `rate_limit_event` and hook output include paths,
  plugin names, account and usage details. Never commit a raw recording. The fixture was cut down
  to the message types the driver reads, and its paths were rewritten.

## 4. Does a pending approval ever time out?

Claude Code: a permission request was left unanswered for 20 minutes.

No timeout. Claude wrote nothing at all for the 1,200 seconds the request stayed open: no
heartbeat, no cancel, no retry. When the request was finally allowed at 1,205 s, it wrote the file
and finished the turn normally, 6 seconds later. That matches the SDK docs ("can stay pending
indefinitely"), so the expiry has to come from Swarn, as PRD §2.6 says: 30 minutes by default, then
an explicit deny logged as `expired`.

Codex: not measured yet. Its approvals go through `codex app-server`, which gets its driver in
week 4, and this section gets the Codex numbers then.
