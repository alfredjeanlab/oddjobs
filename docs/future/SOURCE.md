# Rename `workspace` to `source`

Tech debt: separate code provisioning from isolation before CLOUD_V1.

## Problem

`workspace` conflates two concerns:

1. **Code provisioning** — what files does the job need (git branch, empty dir)
2. **Isolation** — run in a separate directory, not the project root

The name `git = "worktree"` bakes a mechanism (git worktrees) into the
interface. With containers (CLOUD_V1), isolation moves to the container layer,
and code provisioning may use `git clone` instead of `git worktree add`. The
workspace abstraction leaks.

## Change

Rename `workspace` to `source` in runbook definitions. The block describes
where code comes from, not how isolation works.

### Before

```hcl
job "fix" {
  workspace {
    git    = "worktree"
    branch = "fix/${workspace.nonce}"
    ref    = "origin/main"
  }

  step "build" { run = "cargo build" }
}
```

```hcl
job "cleanup" {
  workspace = "folder"

  step "run" { run = "rm -rf /tmp/old-stuff" }
}
```

### After

```hcl
job "fix" {
  source {
    git    = true
    branch = "fix/${source.nonce}"
    ref    = "origin/main"
  }

  step "build" { run = "cargo build" }
}
```

```hcl
job "cleanup" {
  source = "folder"

  step "run" { run = "rm -rf /tmp/old-stuff" }
}
```

No `source` block = run in cwd (project dir). Same as no `workspace` today.

## Template Variable Rename

`${workspace.branch}`, `${workspace.root}`, `${workspace.nonce}` become
`${source.branch}`, `${source.root}`, `${source.nonce}`.

Keep `${workspace.*}` as a deprecated alias during migration.

## `git = true` vs `git = "worktree"`

The old `git = "worktree"` specified the mechanism. The new `git = true` says
"I need a git workspace" — the system picks the mechanism:

- **Local, no container**: `git worktree add` (today's behavior)
- **Local Docker**: `git worktree add` on host, volume-mount into container
- **Fleet / Kubernetes**: `git clone` inside the container

The `branch` and `ref` fields express intent regardless of mechanism.

## Internal Implementation

The engine-internal `Workspace` type, `WorkspaceId`, `WorkspaceStatus`, and
the workspace lifecycle (Creating → Ready → InUse → Cleaning → Deleted) stay
unchanged. This is a runbook-facing rename, not an engine refactor. The
runbook parser maps `source` to the existing workspace machinery.

## Migration

1. Add `source` as an alias for `workspace` in the runbook parser
2. Update all runbooks in `library/` and `.oj/runbooks/` to use `source`
3. Update documentation
4. Deprecation warning when `workspace` is used
5. Remove `workspace` alias in a future release

## Scope

This is a rename and a semantic shift (`git = "worktree"` → `git = true`).
No new functionality. No engine changes. Prepares the runbook vocabulary for
CLOUD_V1 where the provisioning mechanism varies by backend.
