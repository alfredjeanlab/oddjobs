# PreToolUse Prompt Hook: Detect Plan/Question Tools and Transition to Prompting

## Overview

Add a `PreToolUse` hook that detects when an agent invokes `ExitPlanMode`, `EnterPlanMode`, or `AskUserQuestion` and transitions the agent to `MonitorState::Prompting`. The hook is injected into `.claude/settings.json` alongside existing hooks, reads the tool name from stdin JSON, determines the prompt type (`plan_approval` or `question`), and emits an `AgentPrompt` event. This reuses the existing `PromptType` enum, `AgentPrompt` event, `MonitorState::Prompting` state, and `on_prompt` handler (default: `escalate`). The hook does NOT auto-respond — it only detects and transitions state.

## Project Structure

```
crates/
├── cli/src/commands/
│   └── agent.rs           # Add `oj agent hook pretooluse <agent_id>` subcommand
├── engine/src/
│   ├── workspace.rs       # Inject PreToolUse hook into settings
│   └── workspace_tests.rs # Test PreToolUse hook injection
```

No changes needed in `core/src/event.rs` (PromptType already has `PlanApproval` and `Question`), `engine/src/monitor.rs` (already has `Prompting` variant), or event routing (already handles `AgentPrompt`).

## Dependencies

No new external dependencies. Uses existing:
- `serde_json` for hook injection and stdin parsing
- `clap` for CLI subcommand
- Existing `Event::AgentPrompt`, `PromptType`, `MonitorState::Prompting` infrastructure

## Implementation Phases

### Phase 1: Add `oj agent hook pretooluse` CLI Subcommand

Add a new hook subcommand in `crates/cli/src/commands/agent.rs` that handles the PreToolUse hook protocol.

**`HookCommand` enum** — add `Pretooluse` variant:

```rust
pub enum HookCommand {
    Stop { agent_id: String },
    Pretooluse { agent_id: String },
}
```

**`handle_pretooluse_hook` function** — reads stdin JSON, extracts `tool_name`, maps to prompt type, emits event:

```rust
#[derive(Deserialize)]
struct PreToolUseInput {
    tool_name: Option<String>,
}

async fn handle_pretooluse_hook(agent_id: &str, client: &DaemonClient) -> Result<()> {
    let mut input_json = String::new();
    io::stdin().read_to_string(&mut input_json)?;

    let input: PreToolUseInput = serde_json::from_str(&input_json)
        .unwrap_or(PreToolUseInput { tool_name: None });

    let prompt_type = match input.tool_name.as_deref() {
        Some("ExitPlanMode") | Some("EnterPlanMode") => "plan_approval",
        Some("AskUserQuestion") => "question",
        _ => return Ok(()), // unexpected tool, ignore
    };

    let event = Event::AgentPrompt {
        agent_id: AgentId::new(agent_id),
        prompt_type: parse_prompt_type(prompt_type),
    };
    client.emit_event(event).await?;

    Ok(())
}
```

Key details:
- Claude Code PreToolUse hooks receive JSON on stdin with `tool_name` (among other fields)
- The hook should NOT output any blocking response — just emit the event and exit 0
- The `parse_prompt_type` helper already exists in `emit.rs`; either reuse or duplicate the simple match
- If `tool_name` doesn't match our expected tools (shouldn't happen given the matcher), silently no-op

**Milestone:** `cargo build -p oj-cli` succeeds; `oj agent hook pretooluse <id>` parses correctly.

---

### Phase 2: Inject PreToolUse Hook into Agent Settings

Extend `inject_hooks()` in `crates/engine/src/workspace.rs` to add a `PreToolUse` hook entry.

```rust
// In inject_hooks(), after the Notification hooks insertion:

let pretooluse_hook_entry = json!({
    "matcher": "ExitPlanMode|AskUserQuestion|EnterPlanMode",
    "hooks": [{
        "type": "command",
        "command": format!("oj agent hook pretooluse {}", agent_id)
    }]
});

hooks_obj.insert("PreToolUse".to_string(), json!([pretooluse_hook_entry]));
```

The `matcher` field uses pipe-delimited tool names — Claude Code matches against `tool_name` for PreToolUse hooks. This ensures the hook only fires for these three specific tools, not every tool invocation.

**Milestone:** `cargo test -p oj-engine` passes; generated settings contain the PreToolUse hook.

---

### Phase 3: Tests

**`crates/engine/src/workspace_tests.rs`** — Add test for PreToolUse hook injection:

```rust
#[test]
fn prepare_agent_settings_injects_pretooluse_hook() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let agent_id = "test-pretooluse";
    let settings_path = prepare_agent_settings(agent_id, workspace.path(), None).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // PreToolUse hook is present
    assert!(parsed["hooks"]["PreToolUse"].is_array());
    let hooks = parsed["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(hooks.len(), 1);

    // Matcher covers all three tools
    assert_eq!(hooks[0]["matcher"], "ExitPlanMode|AskUserQuestion|EnterPlanMode");

    // Command references the agent hook subcommand
    let inner = hooks[0]["hooks"].as_array().unwrap();
    assert_eq!(inner[0]["type"], "command");
    assert_eq!(
        inner[0]["command"],
        format!("oj agent hook pretooluse {}", agent_id)
    );

    std::env::remove_var("OJ_STATE_DIR");
}
```

Update the existing `prepare_agent_settings_injects_notification_hooks` test if it asserts on the total number of hook categories (it currently doesn't, so likely no change needed).

**`crates/cli/src/commands/agent_tests.rs`** — Add unit test for prompt type mapping:
- `tool_name: "ExitPlanMode"` → `PlanApproval`
- `tool_name: "EnterPlanMode"` → `PlanApproval`
- `tool_name: "AskUserQuestion"` → `Question`
- Missing/unknown `tool_name` → no event emitted

**Milestone:** `make check` passes (fmt, clippy, tests, build, audit, deny).

## Key Implementation Details

### PreToolUse Hook Protocol

Claude Code's PreToolUse hooks:
- Fire **before** a tool is executed
- Receive JSON on stdin containing `tool_name` and tool input details
- Can output JSON to `{"decision": "block", "reason": "..."}` to prevent tool execution
- If they output nothing or `{}`, the tool proceeds normally

We do NOT want to block these tools — we just want to detect them and transition state. The hook emits the event and exits cleanly with no stdout output, allowing the tool to proceed.

### Matcher Format

The `matcher` field for PreToolUse hooks matches against tool names. Pipe-delimited values (`ExitPlanMode|AskUserQuestion|EnterPlanMode`) match any of the listed tools. This is a Claude Code convention — the same pattern used for regex-style matching in other hook types.

### Why a CLI Subcommand Instead of Inline Shell

The instruction suggests using `oj emit agent:prompt --agent {agent_id} --type plan_approval` directly. However, the prompt type depends on which tool triggered the hook (`plan_approval` for ExitPlanMode/EnterPlanMode, `question` for AskUserQuestion). Since the hook command needs to read stdin JSON to determine the tool name, a dedicated `oj agent hook pretooluse` subcommand is the clean approach — matching the existing `oj agent hook stop` pattern.

Alternative considered: a shell one-liner with `jq` to extract `tool_name` and branch. Rejected because it adds a runtime dependency on `jq` and is fragile.

### State Flow

```
Agent calls ExitPlanMode/EnterPlanMode/AskUserQuestion
  → Claude Code fires PreToolUse hook
    → `oj agent hook pretooluse <agent_id>` reads stdin, extracts tool_name
      → Emits Event::AgentPrompt { prompt_type: PlanApproval | Question }
        → Runtime handle_agent_prompt_hook transitions to MonitorState::Prompting
          → Fires on_prompt action (default: escalate)
```

The tool itself still executes (hook doesn't block). The state transition happens in parallel.

### Existing Infrastructure Reused

All downstream handling already exists from the agent-hooks plan:
- `PromptType::PlanApproval` and `PromptType::Question` — already in `core/src/event.rs`
- `Event::AgentPrompt` — already defined and routed
- `MonitorState::Prompting` — already handled in `runtime/monitor.rs`
- `handle_agent_prompt_hook` — already in `runtime/handlers/agent.rs`
- `on_prompt` field on `AgentDef` — already parsed (default: `escalate`)

## Verification Plan

1. **Unit tests** — `cargo test --all` covers hook injection and prompt type mapping
2. **Clippy + fmt** — `cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`
3. **Full gate** — `make check` (fmt, clippy, quench, test, build, audit, deny)
4. **Manual smoke test** — Run a pipeline with an agent that has ExitPlanMode allowed; verify that invoking it triggers `on_prompt` escalation via the PreToolUse hook
