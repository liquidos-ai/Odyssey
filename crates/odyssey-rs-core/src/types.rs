//! Core data types shared across the orchestrator API.

use autoagents_core::agent::{AgentDeriveT, AgentExecutor, AgentHooks};
use chrono::{DateTime, Utc};
use odyssey_rs_config::{ModelConfig, PermissionMode, ToolPolicy};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;
use uuid::Uuid;

/// Unique identifier for a session.
pub type SessionId = Uuid;

/// Message stored in a session transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// Role that produced the message.
    pub role: Role,
    /// Message content.
    pub content: String,
    /// Timestamp for the message.
    pub created_at: DateTime<Utc>,
}

/// Speaker role for a message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System-generated message.
    System,
    /// User-authored message.
    User,
    /// Assistant-authored message.
    Assistant,
}

impl Role {
    /// Return the role as a lowercase string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    /// Parse a role from a lowercase string.
    pub fn parse(value: &str) -> Self {
        if value == "system" {
            Role::System
        } else if value == "assistant" {
            Role::Assistant
        } else {
            Role::User
        }
    }
}

impl FromStr for Role {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Role::parse(value))
    }
}

/// Full session transcript with messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    /// Session identifier.
    pub id: SessionId,
    /// Agent responsible for this session.
    pub agent_id: String,
    /// Ordered list of messages in the session.
    pub messages: Vec<Message>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Summary view of a session for listing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    /// Session identifier.
    pub id: SessionId,
    /// Agent responsible for this session.
    pub agent_id: String,
    /// Count of messages stored.
    pub message_count: usize,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Summary view of a registered agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Agent identifier.
    pub id: String,
    /// Optional human-friendly description.
    pub description: Option<String>,
    /// Optional model configuration override.
    pub model: Option<ModelConfig>,
    /// Tool allow/deny policy for the agent.
    pub tool_policy: ToolPolicy,
    /// Optional permission mode override.
    pub permission_mode: Option<PermissionMode>,
    /// Whether this agent is the default.
    pub is_default: bool,
}

impl From<crate::state::SessionRecord> for Session {
    fn from(record: crate::state::SessionRecord) -> Self {
        Self {
            id: record.id,
            agent_id: record.agent_id,
            created_at: record.created_at,
            messages: record
                .messages
                .into_iter()
                .map(|message| Message {
                    role: Role::parse(&message.role),
                    content: message.content,
                    created_at: message.created_at,
                })
                .collect(),
        }
    }
}

impl From<crate::state::SessionSummaryRecord> for SessionSummary {
    fn from(record: crate::state::SessionSummaryRecord) -> Self {
        Self {
            id: record.id,
            agent_id: record.agent_id,
            message_count: record.message_count,
            created_at: record.created_at,
        }
    }
}

/// Parsed skill content for internal usage.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Skill name.
    pub name: String,
    /// Optional description metadata.
    pub description: Option<String>,
    /// Raw skill body content.
    pub body: String,
    /// Source path for the skill file.
    pub path: PathBuf,
    /// Parsed metadata from frontmatter.
    pub metadata: serde_yaml::Value,
}

pub type LLMProviderID = String;

pub type AgentID = String;

pub trait OdysseyAgentRuntime:
    AgentDeriveT<Output = String> + AgentExecutor + AgentHooks + Clone + Send + Sync + 'static
{
}

impl<T> OdysseyAgentRuntime for T where
    T: AgentDeriveT<Output = String> + AgentExecutor + AgentHooks + Clone + Send + Sync + 'static
{
}

#[cfg(test)]
mod tests {
    use super::{Message, Role, Session};
    use crate::state::{MessageRecord, SessionRecord};
    use chrono::Utc;
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    #[test]
    fn role_parses_and_formats() {
        assert_eq!(Role::parse("system"), Role::System);
        assert_eq!(Role::parse("assistant"), Role::Assistant);
        assert_eq!(Role::parse("user"), Role::User);
        assert_eq!(Role::System.as_str(), "system");
    }

    #[test]
    fn session_from_record_maps_roles() {
        let session_id = Uuid::new_v4();
        let created_at = Utc::now();
        let record = SessionRecord {
            id: session_id,
            agent_id: "agent".to_string(),
            created_at,
            messages: vec![
                MessageRecord {
                    role: "system".to_string(),
                    content: "rules".to_string(),
                    created_at,
                },
                MessageRecord {
                    role: "assistant".to_string(),
                    content: "hello".to_string(),
                    created_at,
                },
            ],
        };

        let session = Session::from(record);
        let expected = Session {
            id: session_id,
            agent_id: "agent".to_string(),
            created_at,
            messages: vec![
                Message {
                    role: Role::System,
                    content: "rules".to_string(),
                    created_at,
                },
                Message {
                    role: Role::Assistant,
                    content: "hello".to_string(),
                    created_at,
                },
            ],
        };
        assert_eq!(session, expected);
    }
}
