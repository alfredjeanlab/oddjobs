# Runbook Concepts — Future Additions

Additions to the runbook primitives described in `docs/concepts/RUNBOOKS.md`.

## ~~Cron Entrypoint~~ (Implemented)

Cron is now implemented. See [Runbook Concepts — Cron](../concepts/RUNBOOKS.md#cron) and [CLI — oj cron](../interface/CLI.md#oj-cron).

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