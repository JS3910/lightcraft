# Local instructions (this fork only; untracked, never sent upstream)

The master copy lives on the fork's `agent-local` branch (`git show origin/agent-local:README.md` explains it). Change it there, push, and copy it into each checkout.

## Agent skills

### Issue tracker

Issues and specs live as GitHub issues on the fork `JS3910/lightcraft`, never on upstream `storytold/lightcraft`. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default triage labels, unchanged. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context. There is no `GLOSSARY.md`; vocabulary comes from the project's own docs (for People and faces, `docs/faces.md`). See `docs/agents/domain.md`.

## Working rules

These are the user's rules for all work on this fork, on every machine. Pass them on in every brief for a subagent.

### The checkout and finished work

- **`main` stays put.** The user builds and runs LightCraft from `main` in the main checkout. Never switch that checkout to another branch.
- **Worktrees.** Ongoing work happens only in git worktrees under `C:\Code\lightcraft-work\wt`, on branches from `main`.
- **Finished work into `main`.** When a piece of work is finished and checked, bring it into `main` in the main checkout. First check that the checkout is clean and that nothing is building or running from it, then tell the user to rebuild.
- **Then push it.** Push `main` to the fork: `git -C <checkout> push origin main`. Push only commits that are verified as finished; check `git log origin/main..main` first, because another session may have committed meanwhile. Never force-push.
- **Upstream is read-only.** Never write to upstream `storytold/lightcraft`: no issues, comments, PRs or pushes.
- **Git steps one at a time.** Before moving a branch, check which branch each checkout has. Do clean-up (deleting a branch, removing a worktree) only as its own step, after the merge it depends on has succeeded.
- **Commit messages.** No issue numbers (`#N`). End each commit with the attribution trailer the session gives.

### Building and testing

- **Test only what changed.** Use `cargo xtask ci --quick`, also before merging work into `main`. Run the full `cargo xtask ci` only when the user asks, or before a pull request to upstream, which needs the user's permission each time.
- **GPU off for tests.** Set `$env:LIGHTCRAFT_GPU = '0'` for every `cargo test` and `cargo xtask ci`, so the GPU tests skip.
  - **Why:** `lightcraft_gpu`'s out-of-memory test fills the card. On Windows, Direct3D 12 spills the overflow into system RAM, and the test took nearly all of a 128 GB machine (fork issue #22).
  - **Running GPU tests:** only when the user asks for it, and never that test.
- **Reuse build caches.** Work done one piece after another shares one warm build folder under `C:\Code\lightcraft-work\targets`, through the same `CARGO_TARGET_DIR`. Separate folders are only for work running at the same time. Never delete a build folder while anything uses it.
- **The computer stays usable.** Run long builds, tests and measurements at below-normal priority: put `(Get-Process -Id $PID).PriorityClass = 'BelowNormal'` on its own line, then run the normal command.
- **Control ports.** Each agent that runs the app uses its own control port in 18000–19999.
- **Optional fonts.** If `CRAFT_FONTS_DIR` isn't set, the build warns that Chinese and Japanese text will show as boxes. Clone `storytold/craft-fonts` next to the checkout and set `CRAFT_FONTS_DIR` as a user environment variable.

### Shell

- **PowerShell only.** Use plain syntax: no Git Bash, no `cmd /c`, no `start`, no quoting tricks.
- **Multi-line text.** Write commit bodies, issue comments and PR bodies to a file with the editor tool, and pass the file (`--body-file`, `-F`).
- **Why:** a `cmd /c 'start "" …'` launch once had its quotes escaped and left a "Windows cannot find '\\'" dialog on the user's screen.

### Subagents

- **Their work is your work.** Never report it as "the agent did it".
- **Check before you trust.** Before merging or reporting a subagent's work, check its CI summary, `git status` and its commits yourself.
