# agent-local: the fork's local agent files

This branch is not LightCraft code. It holds the agent instructions for this fork, which are kept out of the code so they never go upstream to `storytold/lightcraft`. Never merge it into `main`, and never base other branches on it.

| File here | Goes to, in a LightCraft checkout | Excluded from git there by |
|---|---|---|
| `CLAUDE.local.md` | `CLAUDE.local.md` at the root | `.git/info/exclude` |
| `docs/agents/*.md` | `docs/agents/` | `.git/info/exclude` |

## Setting up a machine

In PowerShell, with the checkout at `C:\Code\lightcraft`:

```powershell
git clone https://github.com/JS3910/lightcraft C:\Code\lightcraft
git -C C:\Code\lightcraft remote add upstream https://github.com/storytold/lightcraft
git -C C:\Code\lightcraft remote set-url --push upstream DISABLED_no_push_to_upstream
git -C C:\Code\lightcraft fetch origin agent-local
git -C C:\Code\lightcraft worktree add C:\Code\lightcraft-work\wt\agent-local agent-local
Copy-Item C:\Code\lightcraft-work\wt\agent-local\CLAUDE.local.md C:\Code\lightcraft\
New-Item -ItemType Directory -Force C:\Code\lightcraft\docs\agents
Copy-Item C:\Code\lightcraft-work\wt\agent-local\docs\agents\*.md C:\Code\lightcraft\docs\agents\
Add-Content C:\Code\lightcraft\.git\info\exclude "CLAUDE.local.md"
Add-Content C:\Code\lightcraft\.git\info\exclude "docs/agents/"
git -C C:\Code\lightcraft worktree remove C:\Code\lightcraft-work\wt\agent-local
```

On a machine that already has the checkout, skip the clone and the two `remote` lines.

## Changing the files

Change them on this branch, in a worktree as above. Stage them with `git add -f CLAUDE.local.md docs/agents`: the checkout's `.git/info/exclude` hides them from a plain `git add` here too. Commit and push, then copy them into each checkout again: the `fetch`, `worktree add`, `Copy-Item` and `worktree remove` lines above. A change made only in a checkout's copy stays on that machine.
