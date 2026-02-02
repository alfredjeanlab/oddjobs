# Runbook Concepts — Future Additions

Additions to the runbook primitives described in `docs/01-concepts/RUNBOOKS.md`.

## Cron Entrypoint

Time-driven daemon. Runs a pipeline on a schedule.

```hcl
cron "janitor" {
  interval = "30m"
  run      = { pipeline = "cleanup" }
}
```

Lifecycle: `oj cron enable janitor`, `oj cron disable janitor`, `oj cron run janitor`

Cron fields:
- **interval**: How often to run (e.g., `"30m"`, `"6h"`, `"24h"`)
- **run**: What to execute (`{ pipeline = "name" }`)

Crons are the third entrypoint type alongside commands and workers:

```text
User ─── oj run ───► Command ───► Pipeline (direct)
Queue ──────────────► Worker ────► Pipeline (background)
Timer ──────────────► Cron ──────► Pipeline (scheduled)
```

Use cases range from simple shell-step cleanup (janitor) to agent-driven periodic analysis (security auditor, reliability engineer). See `docs/future/10-runbooks/` for examples.

## ~~Dead Letter Queue~~ (Implemented)

Dead letter semantics with configurable retry are now implemented. See [Runbook Concepts — Queue](../concepts/RUNBOOKS.md#retry-and-dead-letter) and [CLI — oj queue](../interface/CLI.md#oj-queue).

## Nested Pipeline Vars

Pass variables when invoking a nested pipeline from a step:

```hcl
step "deploy" {
  run = { pipeline = "deploy", vars = { ... } }
}
```

Currently, nested pipeline directives are rejected at runtime. The `RunDirective::Pipeline` variant only accepts a `pipeline` name.