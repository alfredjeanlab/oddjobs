# Plan: Route Notifications Through NotifyAdapter

## Overview

The executor in `crates/engine/src/executor.rs` calls `notify_rust` directly in a background thread, bypassing the `NotifyAdapter` trait defined in `crates/adapters/src/notify/`. This plan routes `Effect::Notify` through the `NotifyAdapter` trait, adds a real `DesktopNotifyAdapter` implementation using `notify-rust`, wires `FakeNotifyAdapter` into engine tests, adds notification-specific test coverage, and debugs macOS notification delivery.

## Project Structure

Files to create:
```
crates/adapters/src/notify/desktop.rs   # Real notify-rust implementation
```

Files to modify:
```
crates/adapters/src/notify/mod.rs       # Update trait signature, export DesktopNotifyAdapter
crates/adapters/src/notify/fake.rs      # Update to match new trait signature
crates/adapters/src/notify/noop.rs      # Update to match new trait signature
crates/adapters/Cargo.toml              # Add notify-rust dependency
crates/engine/src/executor.rs           # Add N: NotifyAdapter generic, delegate Effect::Notify
crates/engine/src/executor_tests.rs     # Use FakeNotifyAdapter, verify calls
crates/engine/src/lib.rs                # Update Executor re-export
crates/engine/src/runtime/mod.rs        # Add N generic to RuntimeDeps/Runtime
crates/engine/src/runtime/pipeline.rs   # Update impl bounds
crates/engine/src/runtime/handlers/*.rs # Update impl bounds
crates/engine/src/runtime_tests/mod.rs  # Wire FakeNotifyAdapter, add notify tests
crates/engine/Cargo.toml                # Remove notify-rust dependency
crates/daemon/src/lifecycle.rs          # Pass DesktopNotifyAdapter to RuntimeDeps
crates/daemon/src/lifecycle_tests.rs    # Update RuntimeDeps construction
```

## Dependencies

- `notify-rust = "4"` — moves from `oj-engine` to `oj-adapters` (already in workspace)
- No new external dependencies needed

## Implementation Phases

### Phase 1: Update NotifyAdapter Trait Signature

The current trait uses `send(&self, channel: &str, message: &str)` but `Effect::Notify` carries `title` and `message`. Align the trait to match the effect.

**Files:** `crates/adapters/src/notify/mod.rs`, `fake.rs`, `noop.rs`, `fake_tests.rs`

1. Change the `NotifyAdapter` trait method to:
   ```rust
   async fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError>;
   ```
2. Update `FakeNotifyAdapter` — rename `NotifyCall` fields from `channel`/`message` to `title`/`message`, update the `notify()` impl.
3. Update `NoOpNotifyAdapter` to implement the renamed method.
4. Update `fake_tests.rs` to use the new field names.
5. Run `cargo test -p oj-adapters` to verify.

### Phase 2: Add DesktopNotifyAdapter

Create a real adapter that wraps `notify-rust`, replacing the inline code in the executor.

**Files:** `crates/adapters/src/notify/desktop.rs`, `crates/adapters/src/notify/mod.rs`, `crates/adapters/Cargo.toml`

1. Add `notify-rust.workspace = true` to `crates/adapters/Cargo.toml`.
2. Create `desktop.rs`:
   ```rust
   use super::{NotifyAdapter, NotifyError};
   use async_trait::async_trait;

   #[derive(Clone, Copy, Debug, Default)]
   pub struct DesktopNotifyAdapter;

   impl DesktopNotifyAdapter {
       pub fn new() -> Self {
           Self
       }
   }

   #[async_trait]
   impl NotifyAdapter for DesktopNotifyAdapter {
       async fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError> {
           let title = title.to_string();
           let message = message.to_string();
           // notify_rust::Notification::show() is synchronous on macOS and may
           // block indefinitely in headless environments. Fire-and-forget in a
           // background thread to avoid blocking the async runtime.
           std::thread::spawn(move || {
               tracing::info!(%title, %message, "sending desktop notification");
               match notify_rust::Notification::new()
                   .summary(&title)
                   .body(&message)
                   .show()
               {
                   Ok(_) => {
                       tracing::info!(%title, "desktop notification sent");
                   }
                   Err(e) => {
                       tracing::warn!(%title, error = %e, "desktop notification failed");
                   }
               }
           });
           Ok(())
       }
   }
   ```
3. Register in `mod.rs`: add `mod desktop; pub use desktop::DesktopNotifyAdapter;`
4. Run `cargo test -p oj-adapters` to verify.

### Phase 3: Wire NotifyAdapter Into Executor and Runtime

Add `N: NotifyAdapter` as a generic parameter to `Executor`, `RuntimeDeps`, and `Runtime`. Replace the inline `notify_rust` call with delegation to the adapter.

**Files:** `executor.rs`, `runtime/mod.rs`, `runtime/pipeline.rs`, `runtime/handlers/*.rs`, `lib.rs`

1. Add `N: NotifyAdapter` to `Executor<S, A, C>` → `Executor<S, A, N, C>`:
   ```rust
   pub struct Executor<S, A, N, C: Clock> {
       sessions: S,
       agents: A,
       notifier: N,
       state: Arc<Mutex<MaterializedState>>,
       scheduler: Arc<Mutex<Scheduler>>,
       clock: C,
       event_tx: mpsc::Sender<Event>,
   }
   ```
2. Update `impl` bounds to include `N: NotifyAdapter`.
3. Update `RuntimeDeps<S, A>` → `RuntimeDeps<S, A, N>` with a `notifier: N` field.
4. Update `Runtime<S, A, C>` → `Runtime<S, A, N, C>`.
5. Replace the `Effect::Notify` match arm in `executor.rs`:
   ```rust
   Effect::Notify { title, message } => {
       if let Err(e) = self.notifier.notify(&title, &message).await {
           tracing::warn!(%title, error = %e, "notification send failed");
       }
       Ok(None)
   }
   ```
6. Remove `notify-rust` from `crates/engine/Cargo.toml`.
7. Add `use oj_adapters::NotifyAdapter;` where needed and update all `impl` blocks.
8. Run `cargo check --all` to verify compilation.

### Phase 4: Update Daemon and Test Harnesses

Wire `DesktopNotifyAdapter` in the daemon and `FakeNotifyAdapter` in all test setups.

**Files:** `crates/daemon/src/lifecycle.rs`, `crates/daemon/src/lifecycle_tests.rs`, `crates/engine/src/executor_tests.rs`, `crates/engine/src/runtime_tests/mod.rs`

1. In `lifecycle.rs`: add `DesktopNotifyAdapter::new()` to `RuntimeDeps`:
   ```rust
   RuntimeDeps {
       sessions: session_adapter.clone(),
       agents: agent_adapter,
       notifier: DesktopNotifyAdapter::new(),
       state: Arc::clone(&state),
   }
   ```
2. In `lifecycle_tests.rs`: use `NoOpNotifyAdapter` (or a real `DesktopNotifyAdapter`) to match the daemon test pattern.
3. In `executor_tests.rs`: add `FakeNotifyAdapter` to the test harness:
   ```rust
   type TestExecutor = Executor<FakeSessionAdapter, FakeAgentAdapter, FakeNotifyAdapter, FakeClock>;

   struct TestHarness {
       executor: TestExecutor,
       event_rx: mpsc::Receiver<Event>,
       notifier: FakeNotifyAdapter,
   }
   ```
4. In `runtime_tests/mod.rs`: add `FakeNotifyAdapter` to `TestContext` and `RuntimeDeps`.
5. Run `cargo test --all` to verify.

### Phase 5: Add Notification Tests

Add targeted tests that verify `on_start`, `on_done`, and `on_fail` notifications actually emit via the adapter.

**Files:** `crates/engine/src/executor_tests.rs`, `crates/engine/src/runtime_tests/` (new test module or extend existing)

1. **Executor-level test** — verify `Effect::Notify` delegates to `FakeNotifyAdapter`:
   ```rust
   #[tokio::test]
   async fn notify_effect_delegates_to_adapter() {
       let harness = setup().await;
       harness.executor.execute(Effect::Notify {
           title: "Test".to_string(),
           message: "Hello".to_string(),
       }).await.unwrap();
       let calls = harness.notifier.calls();
       assert_eq!(calls.len(), 1);
       assert_eq!(calls[0].title, "Test");
       assert_eq!(calls[0].message, "Hello");
   }
   ```

2. **Pipeline on_start test** — create a pipeline with `notify.on_start` and verify the notification is emitted during creation. Use a TOML runbook like:
   ```toml
   [pipeline.notified]
   input = ["name"]
   notify = { on_start = "Pipeline ${name} started" }
   [[pipeline.notified.step]]
   name = "init"
   run = "echo ok"
   ```
   After `CommandRun` + event processing, assert `FakeNotifyAdapter.calls()` contains the expected message.

3. **Pipeline on_done test** — run a pipeline to completion and verify `on_done` notification fires. Use `notify = { on_done = "Pipeline ${name} completed" }`.

4. **Pipeline on_fail test** — trigger a pipeline failure and verify `on_fail` notification fires. Use `notify = { on_fail = "Pipeline ${name} failed: ${error}" }`.

5. Run `cargo test --all` to verify all tests pass.

### Phase 6: Debug macOS Notification Delivery

Investigate and fix why desktop notifications don't show on macOS.

**Files:** `crates/adapters/src/notify/desktop.rs`

1. **Add pre-send tracing** — the `DesktopNotifyAdapter` from Phase 2 already includes `tracing::info!` before calling `show()`. Verify this appears in daemon logs with `RUST_LOG=debug`.

2. **Check notification center permissions** — on macOS, notifications require the app to be registered with the Notification Center. `notify-rust` uses `osascript` (AppleScript) on macOS rather than the native `NSUserNotificationCenter` API. Verify:
   - Run `osascript -e 'display notification "test" with title "test"'` manually from the terminal to confirm osascript notifications work.
   - Check System Settings → Notifications → Script Editor (or Terminal) is enabled.
   - If the daemon runs as a launchd service, it may not have access to the user's GUI session. Verify the daemon inherits the user's `DISPLAY` / GUI session environment.

3. **Thread lifetime** — the background thread is fire-and-forget via `std::thread::spawn`. If the process exits before the thread completes, the notification is lost. Add a tracing span that logs when the thread starts and ends to diagnose this.

4. **Consider `terminal-notifier`** — if `osascript` is unreliable, `notify-rust` supports `terminal-notifier` as an alternative backend on macOS. Document findings and any needed configuration in code comments.

5. If the root cause is identified, fix it in `DesktopNotifyAdapter`. If it's a system permission issue, add clear error messages suggesting the user check notification permissions.

## Key Implementation Details

### Trait Signature Alignment

The current `NotifyAdapter::send(channel, message)` doesn't match `Effect::Notify { title, message }`. The trait should use `title`/`message` to match the effect. The `channel` concept can be re-added later if needed for multi-channel routing (Slack, etc.).

### Generic Parameter Ordering

Follow the existing pattern: `Executor<S, A, N, C>` where `S: SessionAdapter`, `A: AgentAdapter`, `N: NotifyAdapter`, `C: Clock`. This keeps the adapter parameters grouped.

### Fire-and-Forget Pattern

The `DesktopNotifyAdapter` spawns a background thread because `notify_rust::Notification::show()` is synchronous and can block on macOS. The adapter returns `Ok(())` immediately. This is intentional — notifications are best-effort and don't produce events.

### Test Isolation

All engine tests use `FakeNotifyAdapter` so that:
- Tests don't trigger real desktop notifications
- Tests can assert on notification content via `FakeNotifyAdapter::calls()`
- Tests are deterministic and don't depend on system notification configuration

## Verification Plan

1. **Unit tests** — `cargo test -p oj-adapters` validates adapter implementations
2. **Engine tests** — `cargo test -p oj-engine` validates executor delegation and pipeline notification lifecycle
3. **Full check** — `make check` runs formatting, clippy, all tests, build, audit, and license checks
4. **Manual verification** — run the daemon with `RUST_LOG=oj_adapters::notify=debug` and trigger a pipeline with `notify.on_start` configured; confirm both log output and desktop notification appearance
