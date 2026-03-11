use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

use crate::engine::core::{EngineError, EngineHandle};
use crate::models::poll::PollProtocol;
use crate::models::{Node, Poll, PollResult};

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub engine: EngineHandle,
    pub started_at: std::time::Instant,
}

// ── Error helpers ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

fn not_found(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (StatusCode::NOT_FOUND, Json(ErrorResponse { error: msg.into() }))
}

fn internal_error(e: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: e.to_string() }))
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: msg.into() }))
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateNodeRequest {
    pub name: String,
    pub addresses: Vec<String>,
    pub polling_profile: String,
    pub parent_node_id: Option<Uuid>,
    pub metadata: Option<HashMap<String, String>>,
    pub tags: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct NodeResponse {
    pub id: Uuid,
    pub name: String,
    pub addresses: Vec<String>,
    pub polling_profile: String,
    pub status: String,
    pub effective_status: String,
    pub parent_node_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
}

impl From<Node> for NodeResponse {
    fn from(n: Node) -> Self {
        Self {
            id: n.id,
            name: n.name,
            addresses: n.addresses,
            polling_profile: n.polling_profile,
            status: format!("{:?}", n.status),
            effective_status: format!("{:?}", n.effective_status),
            parent_node_id: n.parent_node_id,
            tags: n.tags,
            consecutive_failures: n.consecutive_failures,
            consecutive_successes: n.consecutive_successes,
        }
    }
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub status: String,
    pub effective_status: String,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
}

#[derive(Deserialize)]
pub struct CreatePollRequest {
    pub protocol: PollProtocol,
    pub interval_secs: u64,
    pub timeout_ms: u64,
    pub retries: Option<u32>,
    pub failure_threshold: Option<u32>,
    pub recovery_threshold: Option<u32>,
}

#[derive(Serialize)]
pub struct PollResponse {
    pub id: Uuid,
    pub node_id: Uuid,
    pub protocol: PollProtocol,
    pub interval_secs: u64,
    pub timeout_ms: u64,
    pub retries: u32,
    pub failure_threshold: u32,
    pub recovery_threshold: u32,
}

impl From<Poll> for PollResponse {
    fn from(p: Poll) -> Self {
        Self {
            id: p.id,
            node_id: p.node_id,
            protocol: p.protocol,
            interval_secs: p.interval.as_secs(),
            timeout_ms: p.timeout.as_millis() as u64,
            retries: p.retries,
            failure_threshold: p.failure_threshold,
            recovery_threshold: p.recovery_threshold,
        }
    }
}

#[derive(Serialize)]
pub struct PollResultResponse {
    pub id: Uuid,
    pub poll_id: Uuid,
    pub node_id: Uuid,
    pub timestamp: String,
    pub success: bool,
    pub latency_ms: Option<f64>,
    pub error: Option<String>,
}

impl From<PollResult> for PollResultResponse {
    fn from(r: PollResult) -> Self {
        Self {
            id: r.id,
            poll_id: r.poll_id,
            node_id: r.node_id,
            timestamp: r.timestamp.to_rfc3339(),
            success: r.success,
            latency_ms: r.latency.map(|d| d.as_secs_f64() * 1000.0),
            error: r.error,
        }
    }
}

#[derive(Serialize)]
pub struct StatsResponse {
    pub node_count: usize,
    pub engine_uptime_secs: u64,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

pub async fn list_nodes(State(state): State<AppState>) -> ApiResult<Vec<NodeResponse>> {
    let nodes = state.engine.get_all_nodes().await;
    Ok(Json(nodes.into_iter().map(NodeResponse::from).collect()))
}

pub async fn create_node(
    State(state): State<AppState>,
    Json(req): Json<CreateNodeRequest>,
) -> ApiResult<NodeResponse> {
    if req.addresses.is_empty() {
        return Err(bad_request("addresses must not be empty"));
    }
    let mut node = Node::new(&req.name, req.addresses);
    node.polling_profile = req.polling_profile;
    node.parent_node_id = req.parent_node_id;
    node.metadata = req.metadata;
    node.tags = req.tags.unwrap_or_default();
    state.engine.add_node(node.clone()).await.map_err(internal_error)?;
    Ok(Json(NodeResponse::from(node)))
}

pub async fn get_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<NodeResponse> {
    state
        .engine
        .get_node(id)
        .await
        .map(|n| Json(NodeResponse::from(n)))
        .ok_or_else(|| not_found(format!("node {id} not found")))
}

pub async fn delete_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    state.engine.remove_node(id).await.map_err(|e| match e {
        EngineError::NodeNotFound(_) => not_found(format!("node {id} not found")),
        other => internal_error(other),
    })?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

pub async fn get_node_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusResponse> {
    let node = state
        .engine
        .get_node(id)
        .await
        .ok_or_else(|| not_found(format!("node {id} not found")))?;
    Ok(Json(StatusResponse {
        status: format!("{:?}", node.status),
        effective_status: format!("{:?}", node.effective_status),
        consecutive_failures: node.consecutive_failures,
        consecutive_successes: node.consecutive_successes,
    }))
}

pub async fn get_node_results(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<PollResultResponse>> {
    let results = state
        .engine
        .get_poll_results(id, 100)
        .await
        .map_err(internal_error)?;
    Ok(Json(results.into_iter().map(PollResultResponse::from).collect()))
}

pub async fn add_poll(
    State(state): State<AppState>,
    Path(node_id): Path<Uuid>,
    Json(req): Json<CreatePollRequest>,
) -> ApiResult<PollResponse> {
    let poll = Poll {
        id: Uuid::new_v4(),
        node_id,
        protocol: req.protocol,
        interval: Duration::from_secs(req.interval_secs),
        timeout: Duration::from_millis(req.timeout_ms),
        retries: req.retries.unwrap_or(0),
        failure_threshold: req.failure_threshold.unwrap_or(3),
        recovery_threshold: req.recovery_threshold.unwrap_or(1),
    };
    state.engine.add_poll(poll.clone()).await.map_err(|e| match e {
        EngineError::NodeNotFound(_) => not_found(format!("node {node_id} not found")),
        other => internal_error(other),
    })?;
    Ok(Json(PollResponse::from(poll)))
}

pub async fn get_stats(State(state): State<AppState>) -> ApiResult<StatsResponse> {
    let nodes = state.engine.get_all_nodes().await;
    Ok(Json(StatsResponse {
        node_count: nodes.len(),
        engine_uptime_secs: state.started_at.elapsed().as_secs(),
    }))
}
