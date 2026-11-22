# Issue tracker: GitHub (the fork only)

Issues and specs for this work live as GitHub issues on the fork **`JS3910/lightcraft`**. Use the `gh` CLI for all operations.

**Never create, edit, label, comment on or close anything on upstream `storytold/lightcraft`** (issues, PRs, discussions) without the owner's express permission, asked for each time. Reading upstream is fine. In a fork `gh` can resolve to the parent repository, so every command below names the fork with `-R JS3910/lightcraft`; keep that flag on every write.

## Conventions

- **Create an issue**: `gh issue create -R JS3910/lightcraft --title "..." --body "..."`. Use a heredoc or `--body-file` for multi-line bodies.
- **Read an issue**: `gh issue view <number> -R JS3910/lightcraft --json number,title,body,labels,comments`.
- **List issues**: `gh issue list -R JS3910/lightcraft --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Make an issue a sub-issue of a parent**: `gh issue create -R JS3910/lightcraft --parent <parent> ...`, or `gh issue edit <parent> -R JS3910/lightcraft --add-sub-issue <child>` afterwards (`gh` 2.94+). Older `gh`: `gh api --method POST repos/JS3910/lightcraft/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>` (database id, as in **Blocking** below). Without sub-issues, put `Part of #<parent>` at the top of the child body.
- **Comment on an issue**: `gh issue comment <number> -R JS3910/lightcraft --body "..."`
- **Apply / remove labels**: `gh issue edit <number> -R JS3910/lightcraft --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> -R JS3910/lightcraft --comment "..."`

Issue numbers in upstream commit messages, docs and the parity checklist (`#1873`, `#3249`…) are **upstream** numbers. In anything written to the fork (issue bodies, comments, PR descriptions, commit messages pushed there), **never write a linkable reference to an upstream issue or PR**: no `storytold/lightcraft#1873`, no bare `#1873` meant as upstream, no URL to it. GitHub turns each of those into a "mentioned this" entry on the upstream issue, which is a public trace upstream. Write "upstream issue 1873" in plain words instead. A bare `#N` in a fork issue means fork issue N.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

When set to `yes`, PRs run through the same labels and states as issues, using the `gh pr` equivalents:

- **Read a PR**: `gh pr view <number> -R JS3910/lightcraft --comments` and `gh pr diff <number> -R JS3910/lightcraft` for the diff.
- **List external PRs for triage**: `gh api --paginate 'repos/JS3910/lightcraft/pulls?state=open' --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not) | {number, title, author: .user.login, author_association, labels: [.labels[].name]}'`.
- **Comment / label / close**: `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`, each with `-R JS3910/lightcraft`.

A pull request, when one is wanted, is opened **within the fork** (`gh pr create -R JS3910/lightcraft --base main`). `gh pr create` without `-R` offers the parent repository as the base: never accept that without the owner's permission.

GitHub shares one number space across issues and PRs, so a bare `#42` may be either: resolve with `gh pr view 42 -R JS3910/lightcraft` and fall back to `gh issue view 42 -R JS3910/lightcraft`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue on `JS3910/lightcraft`.

## When a skill says "fetch the relevant ticket"

Read it as in **Read an issue** above.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets. All on `JS3910/lightcraft`.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body. `gh issue create -R JS3910/lightcraft --label wayfinder:map`.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue (see **Make an issue a sub-issue of a parent**). Where sub-issues aren't enabled, add the child to a task list in the map body and put `Part of #<map>` at the top of the child body. Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: GitHub's **native issue dependencies**, the canonical, UI-visible representation. Add an edge with `gh api --method POST repos/JS3910/lightcraft/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker's numeric **database id** (`gh api repos/JS3910/lightcraft/issues/<n> --jq .id`, _not_ the `#number` or `node_id`). GitHub reports `issue_dependencies_summary.blocked_by` (open blockers only, the live gate). Where dependencies aren't available, fall back to a `Blocked by: #<n>, #<n>` line at the top of the child body. A ticket is unblocked when every blocker is closed.
- **Frontier query**: list the map's open children (`gh issue list -R JS3910/lightcraft --state open`, scoped to the map's sub-issues / task list), drop any with an open blocker (`issue_dependencies_summary.blocked_by > 0`, or an open issue in the `Blocked by` line) or an assignee; first in map order wins.
- **Claim**: `gh issue edit <n> -R JS3910/lightcraft --add-assignee @me`, the session's first write.
- **Resolve**: `gh issue comment <n> -R JS3910/lightcraft --body "<answer>"`, then `gh issue close <n> -R JS3910/lightcraft`, then append a context pointer to the map's Decisions-so-far.
