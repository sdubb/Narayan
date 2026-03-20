use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    gateway::{gateway::GatewayRequest, LlmGateway},
    providers::ChatResponse,
};

/// A mock gateway that returns pre-queued responses in FIFO order.
/// Useful for deterministic testing without hitting real LLM providers.
pub(crate) struct MockGateway {
    responses: Mutex<Vec<ChatResponse>>,
}

impl MockGateway {
    /// Create a new `MockGateway` with no queued responses.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self { responses: Mutex::new(Vec::new()) }
    }

    /// Create a `MockGateway` pre-loaded with the given responses.
    /// Responses are returned in the order they appear in the vector.
    #[allow(dead_code)]
    pub fn from_responses(responses: Vec<ChatResponse>) -> Self {
        Self { responses: Mutex::new(responses) }
    }

    /// Push a response onto the back of the queue.
    #[allow(dead_code)]
    pub fn push_response(&self, resp: ChatResponse) {
        self.responses.lock().unwrap().push(resp);
    }
}

#[async_trait]
impl LlmGateway for MockGateway {
    async fn chat(&self, _request: GatewayRequest) -> Result<ChatResponse> {
        let mut queue = self.responses.lock().unwrap();
        if queue.is_empty() {
            Ok(ChatResponse { content: None, tool_calls: vec![], input_tokens: 0, output_tokens: 0 })
        } else {
            Ok(queue.remove(0))
        }
    }
}
