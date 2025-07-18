use std::sync::Arc;

use convert_case::{Case, Casing};
use forge_display::TitleFormat;
use forge_domain::{
    ChatRequest, ChatResponse, Event, ToolCallContext, ToolDefinition, ToolName, ToolOutput,
};
use futures::StreamExt;
use tokio::sync::RwLock;

use crate::error::Error;
use crate::{ConversationService, Services, WorkflowService};

pub struct AgentExecutor<S> {
    services: Arc<S>,
    pub tool_agents: Arc<RwLock<Option<Vec<ToolDefinition>>>>,
}

impl<S: Services> AgentExecutor<S> {
    pub fn new(services: Arc<S>) -> Self {
        Self { services, tool_agents: Arc::new(RwLock::new(None)) }
    }

    /// Returns a list of tool definitions for all available agents.
    pub async fn tool_agents(&self) -> anyhow::Result<Vec<ToolDefinition>> {
        if let Some(tool_agents) = self.tool_agents.read().await.clone() {
            return Ok(tool_agents);
        }
        let workflow = self.services.read_merged(None).await?;

        let agents: Vec<ToolDefinition> = workflow.agents.into_iter().map(Into::into).collect();
        *self.tool_agents.write().await = Some(agents.clone());
        Ok(agents)
    }

    /// Executes an agent tool call by creating a new chat request for the
    /// specified agent.
    pub async fn execute(
        &self,
        agent_id: String,
        task: String,
        context: &mut ToolCallContext,
    ) -> anyhow::Result<ToolOutput> {
        context
            .send_text(
                TitleFormat::debug(format!(
                    "{} (Agent)",
                    agent_id.as_str().to_case(Case::UpperSnake)
                ))
                .sub_title(task.as_str()),
            )
            .await?;

        // Create a new conversation for agent execution
        let workflow = self.services.read_merged(None).await?;
        let conversation =
            ConversationService::create_conversation(self.services.as_ref(), workflow).await?;

        // Execute the request through the ForgeApp
        let app = crate::ForgeApp::new(self.services.clone());
        let mut response_stream = app
            .chat(ChatRequest::new(
                Event::new(format!("{agent_id}/user_task_init"), Some(task)),
                conversation.id,
            ))
            .await?;

        // Collect responses from the agent
        while let Some(message) = response_stream.next().await {
            let message = message?;
            match &message {
                ChatResponse::Summary { content } => {
                    return Ok(ToolOutput::text(content));
                }
                _ => {
                    context.send(message).await?;
                }
            }
        }
        Err(Error::EmptyToolResponse.into())
    }

    pub async fn contains_tool(&self, tool_name: &ToolName) -> anyhow::Result<bool> {
        let agent_tools = self.tool_agents().await?;
        Ok(agent_tools.iter().any(|tool| tool.name == *tool_name))
    }
}
