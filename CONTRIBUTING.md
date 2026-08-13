# Contributing to AnkiCollab-Website

Thanks for wanting to help out. This is a small project maintained by one person in their spare
time, so this document exists to keep contributions reviewable and the codebase maintainable —
please read it before opening a PR.

## Start small

**First-time contributors: please open small, focused PRs.** A good first PR fixes one bug, adds
one small page/endpoint improvement, or tweaks one specific thing. This isn't about your skill
level — it's about building trust and shared context incrementally, in both directions. I need to
understand how you write code before reviewing something large, and you'll get a feel for the
codebase's conventions before committing a lot of time to something big.

Concretely:

- **Large, unsolicited refactors will not be accepted.** If you think a module, template, or
  pattern needs a significant rewrite, **open an issue first and discuss it before writing code.**
  I may agree the refactor is worth doing — but I'd rather align on the approach before either of
  us spends hours on a PR that gets rejected in review.
- **Large new features should also be discussed in an issue first**, for the same reason. A big PR
  implementing an entire page/feature I never asked for and haven't reviewed the design of is very
  unlikely to be merged as-is, no matter how well-written it is.
- If you want to tackle something big, the path is: open an issue describing what and why → get
  a thumbs up on the approach → then build it, ideally in reviewable chunks rather than one giant
  PR.

## On AI-assisted code

Using AI tools (Copilot, Claude, ChatGPT, etc.) to help you write code is fine — plenty of us do.
What I can't accept is **code you don't understand or haven't reviewed yourself.**

In practice this means:

- Don't paste a large chunk of AI-generated code straight into a PR without reading, testing, and
  understanding every line of it. If I ask "why did you do it this way?" in review, you should be
  able to answer — it's your PR either way.
- Don't use AI to generate an entire new feature or page wholesale. Large, sprawling,
  "vibe-coded" PRs — inconsistent style, unnecessary abstractions, code that doesn't match how the
  rest of the codebase does things — will be closed, not extensively reviewed and fixed by me. I
  don't have the time to rewrite someone else's PR into something mergeable.
- Small, targeted changes are much easier to verify regardless of how they were written, which is
  another reason to keep PRs small (see above).

If a PR looks AI-generated wholesale and the author can't explain specific design decisions in
it, I'll close it and ask you to resubmit something smaller and in your own words.

## Before you open a PR

1. **Discuss non-trivial changes in an issue first.** Bug fixes and small, obvious improvements
   don't need this — just open the PR. Anything that changes a page layout significantly, touches
   auth/DB schema, or is more than a couple hundred lines should start as an issue.
2. **Run the checks locally before pushing.** CI will run these on every PR, and PRs that don't
   pass won't be merged, so save yourself (and me) a round trip:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets --all-features -- -D warnings -A clippy::nursery
   cargo test --all-features
   ```
   If `cargo fmt --all` doesn't match cleanly, just run it (no `--check`) to auto-fix formatting
   before committing.
3. Keep commits reasonably scoped and write a clear PR description: what changed and why. "Fixes
   #123" is great when there's an issue to reference.

## What's especially welcome right now

- Bug fixes with a clear repro.
- Small, self-contained page or UI improvements.
- Test coverage — we don't have much yet, and small PRs adding tests for existing behavior are an
  easy way to build trust and get familiar with the code.
- Documentation fixes.

## Questions

If anything here is unclear, or you're not sure whether something counts as "small," just ask —
open an issue or drop a question on the [Discord](https://discord.gg/9x4DRxzqwM) before writing
code. That's cheaper for everyone than a rejected PR.
