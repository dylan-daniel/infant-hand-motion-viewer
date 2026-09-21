---
name: commit
description: Commit the current working tree changes as small incremental Conventional Commits. Use when the user asks to commit, or types /commit.
disable-model-invocation: true
---

Commit the pending changes following the rules in AGENTS.md.

1. Run `git status --short` and `git diff` to see everything pending.
2. Group changes into logical units. One unit per commit, never everything at once. If the work spans several features or layers (data, graphics, ui, config), make one commit for each.
3. For each unit, stage only its files with `git add <paths>` (use `git add -p` when a file mixes units), then commit.
4. Message format: `type(scope): short message` or `type: short message`. Lowercase, imperative, one line, no trailing period. Types: feat, fix, refactor, docs, test, chore, build, ci, perf, style, revert. Scope is optional and free-form.
5. Never add a `Co-Authored-By` trailer or any other attribution line to the message.
6. Skip files listed in `.git/info/exclude`.
7. Never push, amend, or force-push. If the pre-commit hook fails, fix the problem and make a new commit.
8. Finish with `git log --oneline` for the new commits and report them.

If the user passes arguments, treat them as a hint for scope or grouping.
