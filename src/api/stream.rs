use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Sse,
    },
};
use futures::stream::{self};

use crate::{events::EventBus, storage::PostgresStore, tenant::model::AuthenticatedTenant};

#[derive(Clone)]
pub struct StreamState {
    pub event_bus: Arc<EventBus>,
    pub store: Arc<PostgresStore>,
}

/// GET /agents/:id/stream
///
/// Server-Sent Events stream for real-time agent progress.
///
/// Events are pushed as:
///   data: {"event":"step_started","agent_id":"...","step_index":0,...}\n\n
///
/// The stream closes automatically when the agent reaches a terminal state
/// (completed, failed) or when the client disconnects.
pub async fn agent_stream(
    State(state): State<StreamState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    // Verify agent belongs to tenant
    match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "agent not found").into_response();
        }
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "store error").into_response();
        }
    }

    let rx = state.event_bus.subscribe(&agent_id);

    // Convert the broadcast receiver into an SSE stream
    let stream = stream::unfold(rx, move |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let is_terminal = matches!(
                        event,
                        crate::events::AgentEvent::GoalComplete { .. }
                            | crate::events::AgentEvent::GoalFailed { .. }
                            | crate::events::AgentEvent::PreflightFailed { .. }
                    );

                    let json = match serde_json::to_string(&event) {
                        Ok(j) => j,
                        Err(_) => continue,
                    };

                    let sse_event = Event::default().data(json);

                    if is_terminal {
                        return Some((Ok::<Event, Infallible>(sse_event), receiver));
                    }

                    return Some((Ok(sse_event), receiver));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return None;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "SSE subscriber lagged");
                    // Emit a lag warning the frontend can surface (event type = "lag")
                    let warn_json = serde_json::to_string(&serde_json::json!({
                        "event":  "lag",
                        "missed": n,
                    }))
                    .unwrap_or_default();
                    let warn_event = Event::default().data(warn_json);
                    return Some((Ok(warn_event), receiver));
                }
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping")).into_response()
}
