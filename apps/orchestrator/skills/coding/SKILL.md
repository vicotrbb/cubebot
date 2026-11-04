---
name: coding
description: How to change software so the change works and stays small. Load this before writing, fixing, refactoring, reviewing or debugging code in any language, including scripts, tests, configuration and build files, even for a change that looks like one line. Not needed to explain code you will not touch.
---

# Coding

You own the outcome, not the diff. A change is finished when you have watched it work, and the next reader can see why every line is there.

## Before you change anything

1. **Restate the goal as something you can check.** "Fix the login bug" becomes "signing in with an expired session redirects to /sign-in, and this test proves it". If you cannot write that sentence, you do not understand the task yet.
2. **Read the code you will touch and its callers.** Look for how this codebase already does the thing: a helper, a pattern, a naming habit. Find the project's own instructions and quality gate (README, CONTRIBUTING, justfile, Makefile, package scripts, CI config) and follow them.
3. **When something is unclear,** state your assumption and carry on if the work is easy to undo. Stop and ask only when the choice is irreversible, reaches outside the project, or two readings would produce very different work.

## While you change it

- **Do what was asked and only that.** Every changed line should trace back to the request. If you notice something else that is wrong, mention it rather than fixing it in passing.
- **Write the least code that fully solves the problem.** No option, abstraction, wrapper or configuration that this task does not use. A helper with one caller is usually just indirection. If it came out at 200 lines and could be 50, rewrite it.
- **Reach for what exists first:** this codebase, then the standard library, then a dependency already installed. Add a new dependency only when writing it yourself would be the larger risk, and pin it the way the project does.
- **Match the surrounding code:** its naming, layout, error style and comment density, even where you would have chosen differently.
- **Clean up only your own mess.** Remove the imports and helpers your change orphaned. Leave existing dead code, and never revert or reformat work you did not do.
- **Let failures show.** No catch-all handler, silent fallback, retry that hides an error, or type escape to quiet a compiler. Handle the errors you can act on and let the rest surface with their cause.
- **Comment why, not what,** and only where the code cannot say it. A known limit you are accepting gets a comment saying what the limit is and when to lift it.
- **Never cut these to save effort:** input validation at trust boundaries, authorization, secrets kept out of code and logs, and anything that could lose someone's data. Use parameterized queries, never string-built ones.

## Fixing a bug

Do not guess. Guess-and-patch is how a one-hour bug becomes a three-day one.

1. **Read the whole error,** including the stack trace and any earlier warning.
2. **Reproduce it,** ideally as a failing test. If you cannot reproduce it, say so and gather evidence; do not ship a theory.
3. **Trace it to where the bad value starts,** not where it crashes. Check what changed recently.
4. **Form one hypothesis and test it with one change.** Fix the cause once, in the shared code, after checking everything that calls it.
5. **After three failed fixes, stop.** Three misses mean your model of the system is wrong. Go back to the evidence, question the design, and tell the person what you have learned.

## Tests

- A test you have never seen fail proves nothing. Watch it fail for the expected reason first. For a regression test, undo the fix and confirm it goes red.
- Test behaviour through the public surface, using real collaborators where you can. A test that only asserts your mock was called is checking the mock.
- Add a test for new logic: a branch, a loop, a parser, anything touching money or security. Trivial changes need none.
- Never weaken, skip or delete a test to get to green unless it is genuinely wrong, and say so when you do.

## Before you say it is done

Claims need evidence you gathered just now:

| Claim                          | What proves it                                             |
| ------------------------------ | ---------------------------------------------------------- |
| Tests pass                     | The test command's output, this run, showing zero failures |
| It builds or type-checks       | That command's exit status, this run                       |
| The bug is fixed               | The reproduction now passes, and failed before the fix     |
| The feature works              | You ran it and saw the behaviour, not only its tests       |
| The other agent or tool did it | You read the resulting diff or output yourself             |

Run the project's whole gate, not just your new test, and read your own diff as a stranger would: anything unrequested, anything left over, anything you would question in review?

Then report plainly: what changed, what you ran and what it showed, what you did not check, and any risk the person should know. If a check failed or could not run, say that in the first line. "Should work" is not a result.

## When you are stuck

If you are re-reading the same files or retrying the same command without learning anything, you are not making progress. Change approach, or report what you tried and exactly what stopped you. Finish or explicitly drop every step you started; never leave work silently half done.
