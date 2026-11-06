---
name: harden
description: >-
  Adversarially review a committed branch with a fresh subagent before it ships, then fix or
  decline each finding, for up to three rounds. Use when a change is committed and about to be
  pushed, or when asked to harden a branch. Skip it only when there is no judgement to review,
  such as a version bump, lockfile or rename. Size is not the test: a one-line change to
  anything important warrants a round.
---

# Harden

The loop is simple: a fresh reviewer without your context reads the branch, you triage what comes back, and you decide whether to go again. Three rounds is the ceiling.

## Establish the ground

Commit first, because the review reads committed history. Then work out the base branch,
preferring `origin/<base>` so a stale local ref doesn't drag other people's commits in, and
check `git diff --stat <base>...HEAD`. An empty diff means the base is wrong, and it is the
dangerous failure: the reviewer reads nothing, reports no findings, and the loop ends looking
clean. On a stacked branch, name the parent branch yourself.

## One round

**1. Dispatch a `general-purpose` subagent.** It gets only its prompt, so the prompt must carry:

- the checkout root and `git diff <base>...HEAD` to review;
- the intent, taken from the ticket or the request, never read off the diff, or the change gets
  judged against itself;
- the full `git log <base>..HEAD` with bodies, since the falsifiable claims live there;
- from round two on, earlier declines with their reasons, marked as settled unless new
  evidence appears;
- an order to report findings only, with file, line, what breaks and a concrete breaking
  input, to say `no findings` rather than pad, and to edit nothing;
- an order not to read or invoke this skill, since knowing the triage rules ends its
  independence;
- that a comment or commit message the code doesn't honour is a finding, and so is a test
  that asserts the implementation instead of the requirement;
- the whole "What gets through" list below, pasted in.

Record `git rev-parse HEAD` before dispatching and check it and `git status --porcelain`
afterwards. A subagent that edits and commits leaves a clean tree, so status alone misses it.
Keep your triage notes outside the repository.

**2. Triage.** Reviewers assert false things, so verify each claim before acting. Then fix it
or decline it with a written reason. Treat each finding as a sample of its class and fix every
instance, not just the one reported. Make one commit per fix and never amend. Decline
out-of-scope findings and note them as follow-ups. Run tests and lints after fixing.

**3. Decide whether to go again.** Three rounds overrides everything here. A round that found
nothing stops the loop. A round where you fixed anything earns another, because nobody has read
those fixes yet. A round where you declined everything earns one more the first time, which
makes disagreeing cost something, and stops the loop the second time in a row.

Before each new round, recompute the diff so it includes your fixes, but keep the original
intent and commit claims. The fix commits are an audit trail, and passing them as claims sends
the reviewer hunting mismatches the loop invented. Make sure lints and tests pass before committing
the changes.

## What gets through

These classes have survived local review before. Hunt them by name.

- **Loose matching.** A pattern matched against text that something else can also produce.
  Ask of every matcher who else can emit this string.
- **Prose-only rules.** A comment or doc states a rule the code doesn't enforce.
- **Guards that accept wrong states.** For example, treating every terminal state but one as
  success, so a cancelled run reads as completed. Enumerate what you accept.
- **First-page reads.** A call without pagination where the wanted item sorts last.
- **Swallowed exit status.** `cmd | jq` reports jq's status, and `2>/dev/null || default`
  turns an auth failure into a plausible answer.
- **Borrowed lists as boundaries.** An allowlist built from a constant that exists for another
  purpose widens silently when someone extends it.
- **Tests that bypass the real gate.** A test asserting through a predicate production doesn't
  call stays green while the real check is broken.
- **Read-modify-write without compare-and-set.** Writing back a whole document loses a
  concurrent change, often a revoke.
- **Identity on rebuilt objects.** Comparing by reference against data that gets cloned,
  serialised or rehydrated.
- **Authorization on the wrong subject.** Checking the record loaded rather than the record
  written, or the caller's id rather than the id in the params.

## When done

Report the rounds run and what ended the loop, what you fixed, and what you declined and why.
If the ceiling stopped the loop, say that the last round's fixes are unreviewed.
