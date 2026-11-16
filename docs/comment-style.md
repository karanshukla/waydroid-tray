# Comment style

A comment is the last resort. Work down this ladder, and write prose only
when all four rungs fail.

1. **Extract it.** Logic that a comment explains belongs in a function with
   a domain name. Arithmetic spelled out in a comment belongs in named
   constants. A coercion that is explained twice belongs in one named helper.

   ```rust
   // Before: inline in the main loop
   if !session_up && state != State::Stopped { State::Stuck } else { state }

   // After
   if session_up { state } else { state.without_session_manager() }
   ```

2. **Name it.** A comment that labels a block (`// write the files`) means
   the block wants to be a function (`write_files`). Sentinels get names too:
   `REPLACE_QUEUED_JOB`, not `"replace" // systemd's default job mode`.

3. **Pin it with a test pair.** A comment that states a business rule is a
   rule nothing enforces. Write one test inside the boundary and one
   outside, named after the rule (`turning_auto_stop_on_restarts_the_countdown`
   / `auto_stop_left_on_keeps_counting_down`), instead of a comment saying
   so. The pair is the point: it fails the day someone changes the rule. In
   a test, a comment that labels the next few asserts is the same smell.
   Split those asserts into their own test, named after what the comment
   said.

4. **Pin cross-boundary rules with an end-to-end test.** What the tray does
   over D-Bus, what it runs as `waydroid`, and what `--install` writes
   belong in `tests/harness/`, not in a paragraph.

5. **Only then, prose.** Hidden constraints, upstream behaviour, protocol
   requirements. State the constraint, not its biography: "Waydroid claims
   its session name before it asks the container to start" is enough
   without the story of how we found out. Measured claims carry a date.

## Linking the test from the code

When rung 3 or 4 replaced a comment, the code it governs says which test
pins the rule, in its doc comment, always with the phrase "pinned by":

```rust
/// The container's state once the session manager has gone: any session
/// it still holds is orphaned (pinned by
/// `tests::a_container_session_without_a_session_manager_is_stuck`,
/// `tests::no_container_session_without_a_session_manager_is_stopped`).
```

A unit test is `` `tests::name` `` (same file) or `` `module::tests::name` ``
(`src/module.rs`). A harness test is `` `harness::module::name` ``
(`tests/harness/module.rs`).

rustdoc can't link to `#[test]` functions (they are compiled out even
under `--cfg test`), so these are code spans, not intra-doc links.
`tests/doc_links.rs` (CI job "Doc Links") checks them instead: every cited
test must be a `#[test]` or `#[tokio::test]` fn in that file.

## Exempt

- `///` and `//!` docs on items, which say what an item is and how to call
  it. They follow the same "constraint, not biography" rule, and shouldn't
  restate the signature.
- `// SAFETY:` comments on `unsafe`, `#[allow]` reasons, and coverage
  pragmas.
