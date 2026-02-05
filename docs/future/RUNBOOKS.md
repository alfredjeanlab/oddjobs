# Runbook Concepts — Future Additions

Additions to the runbook primitives described in `docs/concepts/RUNBOOKS.md`.

## ~~Cron Entrypoint~~ (Implemented)

Cron is now implemented. See [Runbook Concepts — Cron](../concepts/RUNBOOKS.md#cron) and [CLI — oj cron](../interface/CLI.md#oj-cron).

## ~~Dead Letter Queue~~ (Implemented)

Dead letter semantics with configurable retry are now implemented. See [Runbook Concepts — Queue](../concepts/RUNBOOKS.md#retry-and-dead-letter) and [CLI — oj queue](../interface/CLI.md#oj-queue).

## Nested Job Vars

Pass variables when invoking a nested job from a step:

```hcl
step "deploy" {
  run = { job = "deploy", vars = { ... } }
}
```

Currently, nested job directives are rejected at runtime. The `RunDirective::Job` variant only accepts a `job` name.