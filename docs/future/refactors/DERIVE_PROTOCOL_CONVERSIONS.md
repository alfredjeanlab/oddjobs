# Derive Protocol Conversions

The daemon's protocol layer defines Summary and Detail DTO pairs for every
entity (Job, Agent, Worker, Queue, Workspace, etc.) and converts between
domain types and DTOs entirely through hand-written field-by-field copying.
There are zero `From<&Job> for JobSummary` derives -- every conversion is a
manual struct literal with `.clone()` calls on each field. The query
handlers are roughly 40% mechanical field mapping. Replacing this with
derived `From` impls or a conversion macro would eliminate ~1,600-2,200
lines of source and make the protocol layer self-maintaining when domain
types change.

The cost extends into tests: query tests verify that fields were correctly
copied, which becomes unnecessary once conversions are derived. The
listener query files and their tests together account for ~3,500 lines,
a significant fraction of which exists only to shuttle fields between
representations. A `#[derive(IntoSummary)]` or even hand-written `From`
impls in a single location would collapse this.

- `crates/daemon/src/protocol_types.rs` (274 lines) -- 16+ DTO structs (JobSummary, JobDetail, AgentSummary, etc.)
- `crates/daemon/src/protocol_status.rs` (173 lines) -- 14 status/entry DTOs for the status overview
- `crates/daemon/src/protocol.rs` (723 lines) -- Request (43 variants) and Response enums
- `crates/daemon/src/listener/query.rs` (545 lines) -- ~40% is field-by-field mapping to DTOs
- `crates/daemon/src/listener/query_agents.rs` (422 lines) -- agent query with manual field copying
- `crates/daemon/src/listener/query_tests/` -- tests verifying field-copy correctness
- `crates/core/src/job.rs` (618 lines) -- domain Job type; source of truth for fields
- `crates/core/src/agent_run.rs` (246 lines) -- domain AgentRun type

## Acceptance Criteria

- Every Summary/Detail DTO in `protocol_types.rs` and `protocol_status.rs` has a `From<&DomainType>` impl (either derived or centralized)
- Zero field-by-field struct literals remain in `query.rs` and `query_agents.rs` for domain-to-DTO conversion
- Query handler functions call `.into()` or `From::from()` instead of manual field copying
- Adding a field to a domain type produces a compile error if the corresponding DTO conversion is not updated
- Query tests focus on filtering/sorting logic, not field-copy correctness
- All existing tests pass; `make check` is green
- Net reduction of at least 1,000 lines across listener query files and their tests
