use axum::{
    routing::{get, post},
    Router,
};
use tower_http::services::ServeDir;

use crate::api::handlers::{
    add_poll, create_node, delete_node, get_node, get_node_results, get_node_status, get_stats,
    health, list_nodes, AppState,
};

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/nodes", get(list_nodes).post(create_node))
        .route("/api/v1/nodes/{id}", get(get_node).delete(delete_node))
        .route("/api/v1/nodes/{id}/status", get(get_node_status))
        .route("/api/v1/nodes/{id}/results", get(get_node_results))
        .route("/api/v1/nodes/{id}/polls", post(add_poll))
        .route("/api/v1/stats", get(get_stats))
        .with_state(state)
        .fallback_service(ServeDir::new("static"))
}
