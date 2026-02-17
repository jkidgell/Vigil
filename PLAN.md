# Vigil -- Staged Implementation Plan

> **Design spec:** `Network_Monitoring_Engine_Design_v1.1.md` (same directory)
>
> **How to use:** Copy-paste each stage prompt into Claude Code in order (0 → 9).
> Each stage builds on the previous. Every prompt is self-contained -- it
> includes PATH setup, references to the design spec, and verification steps.
>
> **Pre-requisite:** Rust installed via rustup. If `cargo` is not on PATH:
> ```bash
> echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc && source ~/.bashrc
> ```

---

## Stages Overview

| # | Title                    | Spec Sections       | Complexity     | LLM Strategy                              |
|---|--------------------------|---------------------|----------------|--------------------------------------------|
| 0 | Project Scaffolding      | 2, 8                | Routine        | local_llm for all                          |
| 1 | Core Domain Models       | 3.1-3.3, 4 (enums) | Routine+Review | local_llm generates, Claude reviews        |
| 2 | Status Engine            | 4 (complete)        | **Complex**    | Claude directly                            |
| 3 | Dependency Propagation   | 6                   | Medium         | Claude for logic, local_llm for tests      |
| 4 | SQLite Persistence       | 7                   | Mixed          | local_llm for SQL/CRUD, Claude reviews     |
| 5 | Poll Executors           | 3.2, 8, 9           | Mixed          | local_llm for TCP, Claude for ICMP         |
| 6 | Scheduler                | 5 (complete)        | **Complex**    | Claude directly                            |
| 7 | Engine Main Loop         | 2, 4, 5             | **Complex**    | Claude directly                            |
| 8 | Config, CLI, Ops         | 1, 8, 9             | Routine+Review | local_llm for boilerplate                  |
| 9 | API + Smoke Test         | 2, 8, 9             | Mixed          | local_llm for handlers, Claude for arch    |

---

## Stage 0 -- Project Scaffolding

**Covers:** Spec §2 (High-Level Architecture), §8 (Security Invariants)
**Complexity:** Routine
**LLM strategy:** Use `local_llm_*` MCP tools for all generated code (Cargo.toml, main.rs, module stubs). Claude reviews output only.

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Scaffold the Vigil Rust project

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- sections 2
(High-Level Architecture) and 8 (Security Invariants).

**Per CLAUDE.md:** Use local_llm_* MCP tools for all routine boilerplate generation
(Cargo.toml, main.rs, module stubs, directory structure). Review their output
before writing files.

### Requirements

1. **Initialize the project:**
   - `cargo init` in `/home/neuromancer/Projects/Vigil`
   - Edition 2021, name = "vigil"

2. **Set up Cargo.toml with these dependencies** (latest stable versions):
   - `tokio` (full features)
   - `serde` + `serde_json` (derive feature)
   - `uuid` (v4, serde feature)
   - `chrono` (serde feature)
   - `rusqlite` (bundled feature)
   - `tracing` + `tracing-subscriber`
   - `thiserror`
   - `axum` (for future API)
   - `clap` (derive feature)
   - `toml` (for config parsing)

3. **Create the module structure** (empty files with `// TODO` comments):
   ```
   src/
     main.rs          -- tokio::main entry point, tracing init
     lib.rs           -- pub mod declarations
     models/
       mod.rs
       node.rs        -- Node struct
       poll.rs        -- Poll, PollResult structs
       status.rs      -- NodeStatus enum
     engine/
       mod.rs
       status.rs      -- status state machine (Stage 2)
       dependency.rs  -- dependency propagation (Stage 3)
       scheduler.rs   -- scheduler (Stage 6)
       loop.rs        -- main engine loop (Stage 7)
     db/
       mod.rs
       schema.rs      -- SQL schema
       repository.rs  -- CRUD operations
     net/
       mod.rs
       icmp.rs        -- ICMP ping executor
       tcp.rs         -- TCP connect executor
     api/
       mod.rs
       routes.rs      -- axum routes
       handlers.rs    -- request handlers
     config.rs        -- config file parsing
   ```

4. **main.rs** should:
   - Initialize tracing subscriber
   - Print "Vigil starting..." at info level
   - Exit cleanly

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo check 2>&1 | tail -5     # must compile cleanly
cargo run 2>&1 | grep -i vigil  # must print startup message
```
~~~

---

## Stage 1 -- Core Domain Models

**Covers:** Spec §3.1 (Node), §3.2 (Poll), §3.3 (Poll Result), §4 (Status enum only)
**Complexity:** Routine + Review
**LLM strategy:** Use `local_llm_generate_code` for each struct/enum. Claude reviews for correctness against the spec before writing.

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Implement core domain models

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- sections
3.1 (Node), 3.2 (Poll), 3.3 (Poll Result), and the status enum from section 4.

**Per CLAUDE.md:** Use local_llm_generate_code MCP tool for each struct/enum.
Review its output against the spec for correctness before writing to files.

### Requirements

**1. `src/models/status.rs` -- NodeStatus enum**

```rust
// Spec §4: States are Unknown, Up, Down
// Suppressed is an effective_status overlay from §6 (dependency propagation)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Unknown,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectiveStatus {
    Unknown,
    Up,
    Down,
    Suppressed,
}
```

**2. `src/models/node.rs` -- Node struct (spec §3.1)**

Fields:
- `id: Uuid`
- `name: String`
- `addresses: Vec<String>` (IP or hostname)
- `polling_profile: String` (reference to a poll config)
- `status: NodeStatus`
- `effective_status: EffectiveStatus`
- `parent_node_id: Option<Uuid>` (§6: single uplink dependency)
- `metadata: Option<HashMap<String, String>>`
- `tags: Vec<String>`
- `consecutive_failures: u32` (§4: per-node counter)
- `consecutive_successes: u32` (§4: per-node counter)

Derive: Debug, Clone, Serialize, Deserialize. Implement `Node::new(name, addresses)` that
returns a node with `NodeStatus::Unknown`, zero counters, generated UUID.

**3. `src/models/poll.rs` -- Poll and PollResult structs (spec §3.2, §3.3)**

Poll fields:
- `id: Uuid`
- `node_id: Uuid`
- `protocol: PollProtocol` (enum: Icmp, TcpConnect { port: u16 })
- `interval: Duration` (from std::time)
- `timeout: Duration`
- `retries: u32`
- `failure_threshold: u32` (default 3)
- `recovery_threshold: u32` (default 1)

PollResult fields (spec §3.3 -- immutable, timestamped):
- `id: Uuid`
- `poll_id: Uuid`
- `node_id: Uuid`
- `timestamp: chrono::DateTime<chrono::Utc>`
- `success: bool`
- `latency: Option<Duration>`
- `error: Option<String>`

**4. Wire up modules** -- update all `mod.rs` and `lib.rs` so everything is pub
and re-exported.

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo check 2>&1 | tail -5   # must compile with zero errors
cargo test 2>&1               # no tests yet but must not fail to compile
```

Also verify:
- Every field from the spec is present
- NodeStatus has exactly 3 variants (Unknown, Up, Down)
- EffectiveStatus has exactly 4 variants (Unknown, Up, Down, Suppressed)
- PollResult is immutable by convention (no `&mut self` methods)
~~~

---

## Stage 2 -- Status Engine

**Covers:** Spec §4 (complete -- state machine, transitions, determinism rule)
**Complexity:** Complex
**LLM strategy:** Claude implements directly. This is the core state machine and correctness is critical. Do NOT delegate to local_llm.

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Implement the Status Engine (state machine)

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- section 4
(Status Engine) in its entirety. This is the most critical piece of the system.

**Per CLAUDE.md:** This is complex logic -- implement directly, do NOT use
local_llm tools. Correctness is paramount.

### Requirements

Implement in `src/engine/status.rs`:

**1. StatusEngine struct/function** that processes a poll result and returns the
new status. It must be a pure function of:
- current `NodeStatus`
- current `consecutive_failures` and `consecutive_successes`
- `failure_threshold` and `recovery_threshold`
- whether the new poll result is success or failure

**2. Transition rules (spec §4 exactly):**

**On Poll Success:**
- consecutive_successes += 1
- consecutive_failures = 0
- Unknown → Up (after first success, i.e. consecutive_successes >= 1)
- Down → Up (if consecutive_successes >= recovery_threshold)
- Up → Up (no change)

**On Poll Failure:**
- consecutive_failures += 1
- consecutive_successes = 0
- If consecutive_failures >= failure_threshold → Down
- Otherwise: Unknown → Unknown, Up → Up (no change)

**3. Return type** -- a struct `StatusTransition` containing:
- `new_status: NodeStatus`
- `new_consecutive_failures: u32`
- `new_consecutive_successes: u32`
- `changed: bool` (true if status actually transitioned)
- `previous_status: NodeStatus`

**4. Determinism rule (spec §4):** "Status must be reproducible from counters +
thresholds only. No time windows, no decay logic in v1." Enforce this by
ensuring the function takes NO time/clock parameters.

**5. Comprehensive tests** -- write tests in a `#[cfg(test)] mod tests` block:
- Unknown + success → Up
- Unknown + failure (below threshold) → Unknown
- Unknown + failures reaching threshold → Down
- Up + failure (below threshold) → Up
- Up + failures reaching threshold → Down
- Down + success (below recovery threshold) → Down
- Down + successes reaching recovery threshold → Up
- Counter reset on direction change (failures reset on success, vice versa)
- Various threshold values (failure_threshold=1, =5, recovery_threshold=1, =3)

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test engine::status 2>&1   # all tests must pass
cargo check 2>&1 | tail -3       # clean compile
```
~~~

---

## Stage 3 -- Dependency Propagation

**Covers:** Spec §6 (Dependency Propagation -- Simple Uplink Model)
**Complexity:** Medium
**LLM strategy:** Claude implements the propagation logic. Use `local_llm_write_tests` for generating test scaffolding, then Claude reviews/fixes.

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Implement dependency propagation (simple uplink model)

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- section 6
(Dependency Propagation).

**Per CLAUDE.md:** Claude implements the core logic directly. Use
local_llm_write_tests MCP tool to generate initial test scaffolding, then review
and fix tests for correctness.

### Requirements

Implement in `src/engine/dependency.rs`:

**1. Propagation rules (spec §6 exactly):**
- Each node may have `parent_node_id` (single parent only)
- If parent's status == Down → child's effective_status = Suppressed
- The child's internal NodeStatus is NEVER modified by propagation
- Only effective_status is affected
- Suppressed status affects alerting, not polling

**2. Function signature:**
```rust
/// Given a map of all nodes, compute effective_status for a specific node.
/// Walks up the parent chain (single parent only, no deep traversal).
pub fn compute_effective_status(
    node_id: &Uuid,
    nodes: &HashMap<Uuid, Node>,
) -> EffectiveStatus
```

Logic:
- Look up the node. If it has a parent_node_id, check the parent's status.
- If parent status == Down → return EffectiveStatus::Suppressed
- Otherwise → return the node's own status converted to EffectiveStatus
- If node has no parent → return its own status as EffectiveStatus
- Handle missing nodes gracefully (return Unknown)

**3. Batch function:**
```rust
/// Recompute effective_status for all nodes. Returns a Vec of (node_id, new_effective_status).
pub fn recompute_all_effective_statuses(
    nodes: &HashMap<Uuid, Node>,
) -> Vec<(Uuid, EffectiveStatus)>
```

**4. Constraints (spec §6):**
- Single parent only -- no multi-parent logic
- No deep graph traversal (only check immediate parent)
- No cycle detection (config must prevent cycles)

**5. Tests:**
- Node with no parent → effective = own status
- Node with Up parent → effective = own status
- Node with Down parent → effective = Suppressed
- Node whose parent is Unknown → effective = own status (only Down suppresses)
- Missing parent node → effective = own status (graceful fallback)
- Chain: A(Down) → B → verify B is Suppressed but B's internal status unchanged

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test engine::dependency 2>&1   # all tests must pass
cargo check 2>&1 | tail -3
```
~~~

---

## Stage 4 -- SQLite Persistence

**Covers:** Spec §7 (Persistence)
**Complexity:** Mixed
**LLM strategy:** Use `local_llm_generate_code` for SQL schema and basic CRUD. Claude reviews for correctness, WAL mode, and crash safety.

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Implement SQLite persistence layer

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- section 7
(Persistence).

**Per CLAUDE.md:** Use local_llm_generate_code MCP tool for SQL schema strings
and basic CRUD functions. Claude reviews all output for correctness, WAL mode
setup, and crash safety.

### Requirements

**1. `src/db/schema.rs` -- Schema and initialization:**

Tables:
```sql
-- nodes table
CREATE TABLE IF NOT EXISTS nodes (
    id TEXT PRIMARY KEY,           -- UUID as text
    name TEXT NOT NULL,
    addresses TEXT NOT NULL,        -- JSON array
    polling_profile TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'Unknown',
    parent_node_id TEXT,
    metadata TEXT,                  -- JSON object or NULL
    tags TEXT NOT NULL DEFAULT '[]', -- JSON array
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    consecutive_successes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- polls table
CREATE TABLE IF NOT EXISTS polls (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes(id),
    protocol TEXT NOT NULL,         -- JSON: {"Icmp":{}} or {"TcpConnect":{"port":80}}
    interval_secs INTEGER NOT NULL,
    timeout_ms INTEGER NOT NULL,
    retries INTEGER NOT NULL DEFAULT 0,
    failure_threshold INTEGER NOT NULL DEFAULT 3,
    recovery_threshold INTEGER NOT NULL DEFAULT 1
);

-- poll_results table (append-heavy, spec §7)
CREATE TABLE IF NOT EXISTS poll_results (
    id TEXT PRIMARY KEY,
    poll_id TEXT NOT NULL REFERENCES polls(id),
    node_id TEXT NOT NULL REFERENCES nodes(id),
    timestamp TEXT NOT NULL,
    success INTEGER NOT NULL,       -- 0 or 1
    latency_us INTEGER,             -- microseconds, nullable
    error TEXT
);

-- Index for history queries and retention cleanup
CREATE INDEX IF NOT EXISTS idx_poll_results_node_ts ON poll_results(node_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_poll_results_ts ON poll_results(timestamp);
```

**2. Database initialization function:**
- Open/create SQLite database at a configurable path
- Enable WAL mode (`PRAGMA journal_mode=WAL`)
- Enable foreign keys (`PRAGMA foreign_keys=ON`)
- Run schema creation
- Return a connection (or pool)

**3. `src/db/repository.rs` -- CRUD operations:**
- `insert_node(conn, &Node) -> Result<()>`
- `get_node(conn, &Uuid) -> Result<Option<Node>>`
- `get_all_nodes(conn) -> Result<Vec<Node>>`
- `update_node_status(conn, &Uuid, NodeStatus, u32, u32) -> Result<()>`
  (status + counters)
- `delete_node(conn, &Uuid) -> Result<()>`
- `insert_poll(conn, &Poll) -> Result<()>`
- `get_polls_for_node(conn, &Uuid) -> Result<Vec<Poll>>`
- `insert_poll_result(conn, &PollResult) -> Result<()>`
- `get_poll_results(conn, &Uuid, limit: usize) -> Result<Vec<PollResult>>`
- `purge_old_results(conn, retention_days: u32) -> Result<u64>`
  (spec §7: bounded history retention, default 30 days)

**4. Use `rusqlite` with the `bundled` feature.** Serialize/deserialize UUID and
DateTime fields as TEXT. Serialize complex types (addresses, protocol, metadata)
as JSON TEXT.

**5. Tests** (use a temporary in-memory database `:memory:`):
- Insert and retrieve a node
- Update node status and verify counters
- Insert and retrieve poll results
- Purge old results (insert results with old timestamps, verify purge)

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test db:: 2>&1              # all persistence tests pass
cargo check 2>&1 | tail -3
```
~~~

---

## Stage 5 -- Poll Executors

**Covers:** Spec §3.2 (Poll), §8 (Security -- no root), §9 (latency < 50ms)
**Complexity:** Mixed
**LLM strategy:** Use `local_llm_generate_code` for TCP connect executor (straightforward async socket). Claude implements ICMP directly (needs unprivileged approach, platform specifics).

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Implement poll executors (ICMP ping + TCP connect)

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- sections
3.2 (Poll), 8 (no root privileges), and 9 (poll-to-status latency < 50ms).

**Per CLAUDE.md:** Use local_llm_generate_code MCP tool for the TCP connect
executor (routine async socket code). Claude implements the ICMP executor
directly (requires unprivileged approach, platform specifics).

### Requirements

**1. Executor trait** in `src/net/mod.rs`:
```rust
#[async_trait]
pub trait PollExecutor: Send + Sync {
    async fn execute(&self, address: &str, timeout: Duration) -> PollOutcome;
}

pub struct PollOutcome {
    pub success: bool,
    pub latency: Option<Duration>,
    pub error: Option<String>,
}
```

**2. `src/net/tcp.rs` -- TCP Connect executor:**
- Attempt `TcpStream::connect` with tokio and a timeout
- Measure latency (time from connect attempt to success)
- Return PollOutcome with success/failure, latency, and error detail
- Support both IP and hostname addresses
- Port comes from `PollProtocol::TcpConnect { port }`

**3. `src/net/icmp.rs` -- ICMP Ping executor:**
- Spec §8: engine does not require root privileges
- Use the `surge-ping` crate (add to Cargo.toml) which supports unprivileged
  ICMP on Linux (via IPPROTO_ICMP dgram sockets)
- Resolve hostname to IP if needed (use `tokio::net::lookup_host`)
- Send a single ICMP echo request, measure RTT
- Apply timeout
- Return PollOutcome

**4. Retry wrapper** in `src/net/mod.rs`:
```rust
pub async fn execute_with_retries(
    executor: &dyn PollExecutor,
    address: &str,
    timeout: Duration,
    retries: u32,
) -> PollOutcome
```
- On failure, retry up to `retries` times
- Return the first success, or the last failure

**5. Add `surge-ping` and `async-trait` to Cargo.toml.**

**6. Tests:**
- TCP executor: test against a localhost listener (spawn a TcpListener in test)
- TCP executor: test connection refused (connect to a port nothing listens on)
- TCP executor: test timeout (use a non-routable address like 192.0.2.1)
- Retry wrapper: mock executor that fails N times then succeeds
- ICMP: at minimum, a compile-check test (real ICMP may need permissions)

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test net:: 2>&1             # TCP tests must pass
cargo check 2>&1 | tail -3        # ICMP must compile
```

Note: ICMP tests may need `sudo sysctl -w net.ipv4.ping_group_range="0 65535"`
on Linux to allow unprivileged ICMP. Document this in a comment.
~~~

---

## Stage 6 -- Scheduler

**Covers:** Spec §5 (Scheduler Model -- complete)
**Complexity:** Complex
**LLM strategy:** Claude implements directly. The scheduler's drift-prevention and concurrency-bounding logic is critical and must match the spec exactly. Do NOT delegate.

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Implement the async scheduler

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- section 5
(Scheduler Model) in its entirety.

**Per CLAUDE.md:** This is complex async logic with strict determinism
requirements. Implement directly -- do NOT use local_llm tools.

### Requirements

Implement in `src/engine/scheduler.rs`:

**1. ScheduledPoll struct:**
```rust
pub struct ScheduledPoll {
    pub poll: Poll,
    pub next_run_at: Instant,
    pub running: bool,
}
```

**2. Scheduler struct:**
```rust
pub struct Scheduler {
    polls: HashMap<Uuid, ScheduledPoll>,
    max_concurrent: usize,           // spec: recommended 128
    active_count: Arc<AtomicUsize>,   // current running count
    result_tx: mpsc::Sender<PollResult>,
}
```

**3. Execution rules (spec §5 exactly):**

`tick()` method -- called periodically by the engine main loop:
- For each scheduled poll where `now >= next_run_at`:
  - If `running == false` AND `active_count < max_concurrent`:
    - Mark `running = true`
    - Increment `active_count`
    - Spawn async task to execute the poll
    - Set `next_run_at += interval` (NOT `now + interval` -- drift prevention)
  - If `running == true`:
    - Skip execution
    - Set `next_run_at += interval` (still advance to prevent pile-up)
  - If `active_count >= max_concurrent`:
    - Skip (backpressure)

**4. Drift prevention rule (spec §5):**
"Always increment next_run_at by interval (never set to now + interval)."
This is critical. If a poll is late, it catches up by scheduling the next
tick at the original cadence, not relative to when it actually ran.

**5. Task completion:**
- When a spawned poll task finishes, it sends the PollResult through the
  channel (`result_tx`)
- The task also decrements `active_count` and sets `running = false`
- Use a callback or return channel for the scheduler to mark polls as
  not-running

**6. Management methods:**
- `add_poll(poll: Poll)` -- register a new poll
- `remove_poll(poll_id: &Uuid)` -- unregister
- `update_poll(poll: Poll)` -- update interval/settings
- `get_stats() -> SchedulerStats` -- active count, total polls, overdue polls

**7. Concurrency cap:** spec §5 says maximum concurrent polls recommended 128.
Make this configurable via `max_concurrent`.

**8. Tests:**
- Poll fires when next_run_at is reached
- Poll does NOT fire when already running (no overlap)
- next_run_at advances by interval, not now+interval (drift test)
- Concurrency cap prevents exceeding max_concurrent
- Poll results are sent through the channel
- Add/remove/update poll management

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test engine::scheduler 2>&1   # all tests must pass
cargo check 2>&1 | tail -3
```
~~~

---

## Stage 7 -- Engine Main Loop

**Covers:** Spec §2 (Architecture), §4 (Status Engine integration), §5 (Scheduler integration)
**Complexity:** Complex
**LLM strategy:** Claude implements directly. This is the central orchestration point tying together the scheduler, status engine, dependency propagation, and persistence.

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Implement the engine main loop

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- sections 2
(Architecture), 4 (Status Engine), and 5 (Scheduler).

**Per CLAUDE.md:** This is the architectural core -- implement directly, do NOT
use local_llm tools.

### Requirements

Implement in `src/engine/loop.rs` (rename to `src/engine/core.rs` if `loop` is
a keyword issue):

**1. Engine struct:**
```rust
pub struct Engine {
    nodes: HashMap<Uuid, Node>,
    scheduler: Scheduler,
    db: Connection,                   // or pool
    result_rx: mpsc::Receiver<PollResult>,
    shutdown: CancellationToken,      // from tokio_util
}
```
Add `tokio-util` to Cargo.toml (for CancellationToken).

**2. Engine is the single logical writer for state (spec §5):**
"Poll tasks send results via channel to engine. Engine is single logical
writer for state mutation."

All state changes go through the engine loop -- poll executors only send
results back, they never mutate state directly.

**3. Main loop (`run` method):**
```rust
pub async fn run(&mut self) -> Result<()> {
    loop {
        tokio::select! {
            // 1. Receive poll results from channel
            Some(result) = self.result_rx.recv() => {
                self.handle_poll_result(result).await?;
            }
            // 2. Periodic scheduler tick (e.g., every 100ms)
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                self.scheduler.tick().await;
            }
            // 3. Graceful shutdown
            _ = self.shutdown.cancelled() => {
                tracing::info!("Engine shutting down");
                break;
            }
        }
    }
    Ok(())
}
```

**4. `handle_poll_result` method:**
- Look up the node for this result
- Feed the result to the StatusEngine (Stage 2) to compute transition
- If status changed: update node status + counters in memory
- Recompute effective_status via dependency propagation (Stage 3)
- Persist: save poll result to DB, update node status in DB
- Log transitions at info level: "Node {name}: {old} → {new}"

**5. Engine initialization:**
- Load nodes and polls from database
- Initialize scheduler with all polls
- Set up the result channel (bounded, e.g., 1024)

**6. Public API for engine management** (to be called by the API layer later):
- `add_node(&mut self, node: Node) -> Result<()>`
- `remove_node(&mut self, node_id: &Uuid) -> Result<()>`
- `get_node(&self, node_id: &Uuid) -> Option<&Node>`
- `get_all_nodes(&self) -> Vec<&Node>`
- `add_poll(&mut self, poll: Poll) -> Result<()>`
- `get_node_status(&self, node_id: &Uuid) -> Option<(NodeStatus, EffectiveStatus)>`

**7. Thread safety:** The engine itself runs on a single task. External
access (from API handlers) goes through a command channel or
`Arc<RwLock<Engine>>`. Choose the command-channel approach for cleaner
separation.

Define `EngineCommand` enum + `EngineHandle` for sending commands:
```rust
pub enum EngineCommand {
    AddNode { node: Node, reply: oneshot::Sender<Result<()>> },
    RemoveNode { node_id: Uuid, reply: oneshot::Sender<Result<()>> },
    GetNode { node_id: Uuid, reply: oneshot::Sender<Option<Node>> },
    GetAllNodes { reply: oneshot::Sender<Vec<Node>> },
    AddPoll { poll: Poll, reply: oneshot::Sender<Result<()>> },
    GetStatus { node_id: Uuid, reply: oneshot::Sender<Option<(NodeStatus, EffectiveStatus)>> },
    Shutdown,
}
```

**8. Integration with shutdown:** handle SIGTERM/SIGINT via tokio signal,
trigger the CancellationToken.

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo check 2>&1 | tail -5       # must compile cleanly
cargo test engine:: 2>&1          # all engine tests pass (stages 2, 3, 6, 7)
```
~~~

---

## Stage 8 -- Config, CLI, Ops

**Covers:** Spec §1 (Philosophy), §8 (Security), §9 (Success Criteria)
**Complexity:** Routine + Review
**LLM strategy:** Use `local_llm_generate_code` / `local_llm_generate_boilerplate` for config struct, TOML parsing, and clap CLI. Claude reviews.

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Implement configuration, CLI, and operational setup

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- sections 1
(Philosophy), 8 (Security), and 9 (Success Criteria).

**Per CLAUDE.md:** Use local_llm_generate_code and local_llm_generate_boilerplate
MCP tools for config struct, TOML parsing boilerplate, and clap CLI definition.
Claude reviews all output.

### Requirements

**1. `src/config.rs` -- Configuration:**

```toml
# Example vigil.toml
[engine]
db_path = "/var/lib/vigil/vigil.db"
max_concurrent_polls = 128
tick_interval_ms = 100
retention_days = 30

[api]
bind_address = "127.0.0.1:3030"    # spec §8: localhost by default

[logging]
level = "info"                      # trace, debug, info, warn, error
```

Define a `VigilConfig` struct with `serde::Deserialize` that maps to this
TOML structure. Provide sensible defaults for all fields.

Load config from:
1. Default path: `/etc/vigil/vigil.toml`
2. CLI-specified path (overrides default)
3. Environment variable overrides: `VIGIL_DB_PATH`, `VIGIL_BIND_ADDRESS`, etc.

**2. `src/main.rs` -- CLI with clap:**

```
vigil [OPTIONS]

Options:
  -c, --config <PATH>    Config file path [default: /etc/vigil/vigil.toml]
  -d, --db <PATH>        Database path (overrides config)
  -b, --bind <ADDR>      API bind address (overrides config)
  -v, --verbose          Increase log verbosity (repeatable)
  -V, --version          Print version
  -h, --help             Print help
```

**3. Update main.rs to:**
- Parse CLI args with clap
- Load config file (with fallback to defaults if file doesn't exist)
- Apply CLI overrides
- Initialize tracing with configured level
- Initialize database (from Stage 4)
- Log startup configuration at info level
- Start the engine (from Stage 7)
- Handle graceful shutdown

**4. Create an example config file** at `config/vigil.example.toml` with
comments explaining each option.

**5. Spec §8 compliance checks:**
- API binds localhost by default ✓ (config default)
- No root required ✓ (architecture)
- DB path configurable ✓

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -3
# Test CLI help
cargo run -- --help 2>&1 | head -15
# Test with default config (should start and not crash)
cargo run -- -c /dev/null 2>&1 | head -5
# Test version flag
cargo run -- --version 2>&1
```
~~~

---

## Stage 9 -- API + Smoke Test

**Covers:** Spec §2 (API/IPC layer), §8 (Security), §9 (Success Criteria)
**Complexity:** Mixed
**LLM strategy:** Use `local_llm_generate_code` for axum route/handler boilerplate. Claude architects the API design and wires it into the engine.

### Prompt

~~~markdown
export PATH="$HOME/.cargo/bin:$PATH"

## Task: Implement the REST API and run a smoke test

Read the design spec at `Network_Monitoring_Engine_Design_v1.1.md` -- sections 2
(Architecture: "API / IPC"), 8 (Security: localhost default), and 9 (Success
Criteria).

**Per CLAUDE.md:** Use local_llm_generate_code and local_llm_generate_boilerplate
MCP tools for axum route definitions and handler boilerplate. Claude handles the
architectural wiring (connecting handlers to EngineHandle from Stage 7) and
reviews all output.

### Requirements

**1. `src/api/routes.rs` -- Define routes:**

```
GET  /api/v1/health              -- Health check
GET  /api/v1/nodes               -- List all nodes
POST /api/v1/nodes               -- Add a node
GET  /api/v1/nodes/:id           -- Get single node
DELETE /api/v1/nodes/:id         -- Remove a node
GET  /api/v1/nodes/:id/status    -- Get node status + effective_status
GET  /api/v1/nodes/:id/results   -- Get recent poll results for node
POST /api/v1/nodes/:id/polls     -- Add a poll to a node
GET  /api/v1/stats               -- Engine stats (node count, active polls, etc.)
```

**2. `src/api/handlers.rs` -- Implement handlers:**

Each handler receives the `EngineHandle` (from Stage 7) as axum state and sends
commands through the command channel. All responses are JSON.

Request/response types:
- `CreateNodeRequest { name, addresses, polling_profile, parent_node_id?, metadata?, tags? }`
- `NodeResponse` (serialized Node)
- `StatusResponse { status, effective_status, consecutive_failures, consecutive_successes }`
- `CreatePollRequest { protocol, interval_secs, timeout_ms, retries?, failure_threshold?, recovery_threshold? }`
- `StatsResponse { node_count, active_polls, engine_uptime }`

Error handling: return proper HTTP status codes (404 for not found, 400 for
bad request, 500 for internal errors). Use a consistent error response format:
`{ "error": "message" }`

**3. Wire into main.rs:**
- Start the axum server on the configured bind address
- Share EngineHandle with all routes via axum state
- Run API server and engine loop concurrently (tokio::select or spawn)
- Both shut down together on SIGTERM/SIGINT

**4. Spec §8 compliance:**
- Bind to 127.0.0.1 by default
- No authentication in v1 (noted as TODO for future)

**5. Smoke test script** -- create `tests/smoke_test.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail
BASE="http://127.0.0.1:3030/api/v1"

echo "=== Health check ==="
curl -sf "$BASE/health" | jq .

echo "=== Add a node ==="
NODE=$(curl -sf -X POST "$BASE/nodes" \
  -H 'Content-Type: application/json' \
  -d '{"name":"test-router","addresses":["127.0.0.1"],"polling_profile":"default"}')
echo "$NODE" | jq .
NODE_ID=$(echo "$NODE" | jq -r '.id')

echo "=== Get all nodes ==="
curl -sf "$BASE/nodes" | jq .

echo "=== Get node status ==="
curl -sf "$BASE/nodes/$NODE_ID/status" | jq .

echo "=== Add a TCP poll ==="
curl -sf -X POST "$BASE/nodes/$NODE_ID/polls" \
  -H 'Content-Type: application/json' \
  -d '{"protocol":{"TcpConnect":{"port":22}},"interval_secs":30,"timeout_ms":5000}'  | jq .

echo "=== Wait for a poll cycle ==="
sleep 35

echo "=== Check status after poll ==="
curl -sf "$BASE/nodes/$NODE_ID/status" | jq .

echo "=== Get poll results ==="
curl -sf "$BASE/nodes/$NODE_ID/results" | jq .

echo "=== Engine stats ==="
curl -sf "$BASE/stats" | jq .

echo "=== Delete node ==="
curl -sf -X DELETE "$BASE/nodes/$NODE_ID" | jq .

echo "=== SMOKE TEST PASSED ==="
```

**6. Integration test** in `tests/integration.rs` (Rust):
- Start engine + API in a background tokio task
- Use `reqwest` (add as dev-dependency) to hit API endpoints
- Create a node, verify it appears in list
- Check health endpoint
- Clean shutdown

### Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -3                    # clean build
cargo test 2>&1                                # all unit + integration tests pass

# Manual smoke test (in separate terminal):
# Terminal 1: cargo run
# Terminal 2: bash tests/smoke_test.sh
```

---

## Final verification (after all stages):

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test 2>&1                 # all tests pass
cargo clippy 2>&1               # no warnings
cargo build --release 2>&1      # release build succeeds

# Check binary size and startup
ls -lh target/release/vigil
./target/release/vigil --version
./target/release/vigil --help
```
~~~

---

## Appendix: Dependency Graph

```
Stage 0 (Scaffold)
  └→ Stage 1 (Models)
       ├→ Stage 2 (Status Engine)
       ├→ Stage 3 (Dependency) ──────────────┐
       ├→ Stage 4 (SQLite Persistence)       │
       └→ Stage 5 (Poll Executors)           │
            └→ Stage 6 (Scheduler)           │
                 └→ Stage 7 (Engine Loop) ←──┘
                      └→ Stage 8 (Config/CLI)
                           └→ Stage 9 (API + Smoke Test)
```

Stages 2-5 can be done in any order after Stage 1. Stage 6 requires Stage 5.
Stage 7 requires Stages 2, 3, 4, 5, and 6. Stages 8 and 9 are sequential after 7.
