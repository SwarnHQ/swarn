# License audit

Week 1 deliverable from the technical PRD (§1.5, §8). Last run 16 Oct 2026 against `Cargo.lock` at
the week 1 scaffold.

## Result

No GPL, AGPL or LGPL code ships in any Swarn binary. 376 third-party crates are linked into, or
built for, the macOS arm64 binaries (normal and build dependencies; dev-only crates are excluded).
374 of them are under permissive licenses. The other two are MPL-2.0, and both were reviewed:

| Crate | Pulled in by | Why it is acceptable |
| --- | --- | --- |
| `cbindgen` 0.28.0 | build dependency of `gpui-pre-apple` | Generates C headers at build time. None of its code ends up in our binaries. |
| `option-ext` 0.2.0 | `dirs-sys` ← `dirs` ← `zed-font-kit` ← `gpui-pre-apple` | MPL-2.0 is file-level copyleft. We use the crate unmodified, so the only obligation is that its source stays available, which crates.io already provides. |

## The Zed question

The PRD flagged Zed's crates as the main risk, because some of them (`ui`, `editor`, the markdown
crates) are GPL-3.0. Here is what we actually depend on:

- All 15 Zed-derived crates come from the `gpui-pre` 0.3.9 snapshot of zed@93d7456: `gpui-pre`,
  `-apple`, `-collections`, `-derive-refineable`, `-http-client`, `-macos`, `-macros`, `-platform`,
  `-refineable`, `-scheduler`, `-shared-string`, `-sum-tree`, `-util`, `-ztracing` and
  `-ztracing-macro`. Each declares Apache-2.0 and ships `LICENSE-APACHE`.
- `sum_tree`, which the PRD listed as likely GPL, is Apache-2.0 and is already in the tree as a gpui
  dependency.
- `zed-font-kit` is MIT OR Apache-2.0.
- Nothing from Zed's GPL crates is in the graph.

`gpui-component` 0.7.3 depends on the same `gpui-pre =0.3.9`, so adding it later won't bring in a
second gpui. Re-run the audit when it lands, because it also pulls in `gpui-fast`, `gpui-base` and
`gpui-kit-assets`.

## Fonts

Manrope and Roboto Mono are SIL OFL 1.1. See `crates/swarn-desktop/assets/fonts/README.md`. The OFL
text itself still has to be added there before the first release.

## Re-running it

```bash
python3 -I scripts/license_audit.py
```

The script walks `cargo metadata` and exits non-zero if any crate has a license expression that
isn't fully permissive and isn't in its reviewed list. Adding a crate to that list is a decision, so
the entry has to say why. It belongs in CI next to the tests. `cargo-deny` would do the same job
with more features, and it's worth switching to once the CI runner can install it (PRD review item
18).
