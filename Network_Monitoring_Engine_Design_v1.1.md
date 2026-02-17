# Network Monitoring Engine -- Design Specification (v1.1)

## 1. Purpose & Philosophy

This project is a modern, lightweight replacement for MikroTik The Dude.

Core goals:

-   Headless-first (daemon, not desktop-first)
-   Extremely low resource usage
-   Deterministic, predictable behavior
-   Clear separation between engine and UI
-   Designed for always-on operation
-   Scalable to 600+ nodes without architectural change

The system must function fully without a UI. Any UI is a client of the
engine, not a dependency.

------------------------------------------------------------------------

## 2. High-Level Architecture

\[ Monitoring Engine (Rust daemon) \] ↑ API / IPC ↓ \[ UI Clients
(optional) \]

The engine is authoritative. UI clients are disposable.

------------------------------------------------------------------------

## 3. Core Concepts & Domain Model

### 3.1 Node

Represents a monitored entity.

Required: - id (UUID) - name - addresses (IP / hostname) -
polling_profile - status

Optional: - parent_node_id (single uplink dependency) - metadata - tags

------------------------------------------------------------------------

### 3.2 Poll

Defines a scheduled check.

Each poll includes: - protocol - interval - timeout - retries -
failure_threshold (default 3) - recovery_threshold (default 1)

------------------------------------------------------------------------

### 3.3 Poll Result

Immutable, timestamped outcome of a poll. Contains: - success (bool) -
latency (optional) - error (optional)

Results are appended to history.

------------------------------------------------------------------------

## 4. Status Engine (Formal State Machine)

States: - Unknown - Up - Down

Per-node counters: - consecutive_failures - consecutive_successes

Initial state: status = Unknown consecutive_failures = 0
consecutive_successes = 0

### On Poll Success

consecutive_successes += 1\
consecutive_failures = 0

Transitions: - Unknown → Up (after first success) - Down → Up (if
consecutive_successes \>= recovery_threshold) - Up → Up

### On Poll Failure

consecutive_failures += 1\
consecutive_successes = 0

Transitions: - If consecutive_failures \>= failure_threshold → Down -
Otherwise: - Unknown → Unknown - Up → Up

Determinism rule: Status must be reproducible from counters + thresholds
only. No time windows, no decay logic in v1.

------------------------------------------------------------------------

## 5. Scheduler Model (Async, Deterministic, Bounded)

Goals: - Fixed intervals - No overlapping polls per node -
Drift-resistant - Bounded concurrency

Each poll maintains: - interval - next_run_at - running (bool)

### Execution Rules

If now \>= next_run_at AND running == false: - Spawn async task - Set
running = true - next_run_at += interval

If now \>= next_run_at AND running == true: - Skip execution -
next_run_at += interval

Drift prevention rule: Always increment next_run_at by interval (never
set to now + interval).

Global concurrency cap: Maximum concurrent polls (recommended: 128).

Poll tasks send results via channel to engine. Engine is single logical
writer for state mutation.

------------------------------------------------------------------------

## 6. Dependency Propagation (Simple Uplink Model)

Each node may define: parent_node_id (single parent only)

Rules: - If parent status == Down → child effective_status =
Suppressed - Internal node status remains unchanged - Only effective
status affects alerting

Constraints: - Single parent only - No multi-parent logic - No deep
graph traversal - No automatic cycle detection (config must prevent
cycles)

------------------------------------------------------------------------

## 7. Persistence

-   SQLite
-   WAL mode enabled
-   Append-heavy design
-   Crash-safe
-   Bounded history retention (default 30 days)

------------------------------------------------------------------------

## 8. Security Invariants

-   Engine does not require root privileges
-   API binds to localhost by default
-   TLS required for remote communication
-   No plaintext credential storage

------------------------------------------------------------------------

## 9. Success Criteria

-   Stable operation for weeks
-   RAM target: \< 64MB for \~100 nodes
-   Scales to 600+ nodes without redesign
-   Poll-to-status latency \< 50ms
-   Deterministic behavior under failure storms

------------------------------------------------------------------------

This document is the authoritative specification for v1.1. Any
implementation violating these rules is considered a bug.
