use crate::agent::emit;
use odyssey_rs_protocol::{
    ApprovalDecision, EventMsg, EventPayload, PermissionAction, PermissionRequest,
};
use odyssey_rs_tools::ToolError;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct ApprovalStore {
    inner: Arc<Mutex<ApprovalState>>,
}

#[derive(Default)]
struct ApprovalState {
    pending: HashMap<Uuid, PendingApproval>,
    always_allow_tools: HashMap<Uuid, Vec<String>>,
}

struct PendingApproval {
    session_id: Uuid,
    turn_id: Uuid,
    sender: oneshot::Sender<ApprovalDecision>,
}

impl ApprovalStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn request_tool(
        &self,
        session_id: Uuid,
        turn_id: Uuid,
        tool_name: &str,
        sender: broadcast::Sender<EventMsg>,
    ) -> Result<(), ToolError> {
        if self.is_always_allowed(session_id, tool_name) {
            return Ok(());
        }

        let request_id = Uuid::new_v4();
        let (decision_tx, decision_rx) = oneshot::channel();
        {
            let mut state = self.inner.lock();
            state.pending.insert(
                request_id,
                PendingApproval {
                    session_id,
                    turn_id,
                    sender: decision_tx,
                },
            );
        }

        emit(
            &sender,
            session_id,
            EventPayload::PermissionRequested {
                turn_id,
                request_id,
                action: PermissionAction::Ask,
                request: PermissionRequest::Tool {
                    name: tool_name.to_string(),
                },
            },
        );

        let decision = decision_rx.await.map_err(|_| {
            ToolError::PermissionDenied(format!("approval request {request_id} was dropped"))
        })?;

        if matches!(decision, ApprovalDecision::AllowAlways) {
            self.store_allow_always(session_id, tool_name);
        }

        match decision {
            ApprovalDecision::AllowOnce | ApprovalDecision::AllowAlways => Ok(()),
            ApprovalDecision::Deny => Err(ToolError::PermissionDenied(format!(
                "tool {tool_name} was denied"
            ))),
        }
    }

    pub fn resolve(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        sender: broadcast::Sender<EventMsg>,
    ) -> bool {
        let pending = {
            let mut state = self.inner.lock();
            state.pending.remove(&request_id)
        };
        let Some(pending) = pending else {
            return false;
        };
        let _ = pending.sender.send(decision);
        emit(
            &sender,
            pending.session_id,
            EventPayload::ApprovalResolved {
                turn_id: pending.turn_id,
                request_id,
                decision,
            },
        );
        true
    }

    pub fn session_id_for_request(&self, request_id: Uuid) -> Option<Uuid> {
        self.inner
            .lock()
            .pending
            .get(&request_id)
            .map(|pending| pending.session_id)
    }

    fn is_always_allowed(&self, session_id: Uuid, tool_name: &str) -> bool {
        self.inner
            .lock()
            .always_allow_tools
            .get(&session_id)
            .is_some_and(|tools| tools.iter().any(|tool| tool == tool_name))
    }

    fn store_allow_always(&self, session_id: Uuid, tool_name: &str) {
        let mut state = self.inner.lock();
        let tools = state.always_allow_tools.entry(session_id).or_default();
        if !tools.iter().any(|tool| tool == tool_name) {
            tools.push(tool_name.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn stores_allow_always_for_future_requests() {
        let approvals = ApprovalStore::new();
        let (sender, _) = broadcast::channel(8);
        let session_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let cloned = approvals.clone();
        let sender_clone = sender.clone();

        let waiter = tokio::spawn(async move {
            cloned
                .request_tool(session_id, turn_id, "Bash", sender_clone)
                .await
        });

        let mut events = sender.subscribe();
        let request = events.recv().await.expect("permission event");
        let request_id = match request.payload {
            EventPayload::PermissionRequested { request_id, .. } => request_id,
            other => panic!("unexpected event: {other:?}"),
        };
        assert!(approvals.resolve(request_id, ApprovalDecision::AllowAlways, sender.clone()));
        assert!(waiter.await.expect("join").is_ok());

        assert!(approvals.is_always_allowed(session_id, "Bash"));
    }
}
