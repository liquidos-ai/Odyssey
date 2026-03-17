use crate::agent::emit;
use crate::session::ApprovalStore;
use odyssey_rs_protocol::{EventMsg, EventPayload, ExecStream};
use odyssey_rs_tools::{ToolApprovalHandler, ToolEvent, ToolEventSink};
use tokio::sync::broadcast;
use uuid::Uuid;

pub(crate) struct RuntimeToolEventSink {
    pub session_id: Uuid,
    pub turn_id: Uuid,
    pub sender: broadcast::Sender<EventMsg>,
    pub working_dir: String,
}

impl ToolEventSink for RuntimeToolEventSink {
    fn emit(&self, event: ToolEvent) {
        let payload = match event {
            ToolEvent::CommandStarted {
                exec_id, command, ..
            } => EventPayload::ExecCommandBegin {
                turn_id: self.turn_id,
                exec_id,
                command,
                cwd: Some(self.working_dir.clone()),
            },
            ToolEvent::CommandStdout { exec_id, line, .. } => {
                EventPayload::ExecCommandOutputDelta {
                    turn_id: self.turn_id,
                    exec_id,
                    stream: ExecStream::Stdout,
                    delta: line,
                }
            }
            ToolEvent::CommandStderr { exec_id, line, .. } => {
                EventPayload::ExecCommandOutputDelta {
                    turn_id: self.turn_id,
                    exec_id,
                    stream: ExecStream::Stderr,
                    delta: line,
                }
            }
            ToolEvent::CommandFinished {
                exec_id, status, ..
            } => EventPayload::ExecCommandEnd {
                turn_id: self.turn_id,
                exec_id,
                exit_code: status,
            },
        };
        emit(&self.sender, self.session_id, payload);
    }
}

#[derive(Clone)]
pub(crate) struct RuntimeApprovalHandler {
    pub session_id: Uuid,
    pub turn_id: Uuid,
    pub sender: broadcast::Sender<EventMsg>,
    pub approvals: ApprovalStore,
}

#[async_trait::async_trait]
impl ToolApprovalHandler for RuntimeApprovalHandler {
    async fn request_tool_approval(&self, tool: &str) -> Result<(), odyssey_rs_tools::ToolError> {
        self.approvals
            .request_tool(self.session_id, self.turn_id, tool, self.sender.clone())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeApprovalHandler, RuntimeToolEventSink};
    use crate::session::ApprovalStore;
    use odyssey_rs_protocol::{ApprovalDecision, EventPayload, ExecStream};
    use odyssey_rs_tools::{ToolApprovalHandler, ToolEvent, ToolEventSink};
    use pretty_assertions::assert_eq;
    use tokio::sync::broadcast;
    use uuid::Uuid;

    #[test]
    fn sink_emits_exec_events_with_expected_payloads() {
        let (sender, mut receiver) = broadcast::channel(8);
        let session_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let exec_id = Uuid::new_v4();
        let sink = RuntimeToolEventSink {
            session_id,
            turn_id,
            sender,
            working_dir: "/workspace/demo".to_string(),
        };

        sink.emit(ToolEvent::CommandStarted {
            tool: "Bash".to_string(),
            exec_id,
            command: vec!["echo".to_string(), "hi".to_string()],
        });
        sink.emit(ToolEvent::CommandStdout {
            tool: "Bash".to_string(),
            exec_id,
            line: "out".to_string(),
        });
        sink.emit(ToolEvent::CommandStderr {
            tool: "Bash".to_string(),
            exec_id,
            line: "err".to_string(),
        });
        sink.emit(ToolEvent::CommandFinished {
            tool: "Bash".to_string(),
            exec_id,
            status: 7,
        });

        let begin = receiver.try_recv().expect("begin event");
        let stdout = receiver.try_recv().expect("stdout event");
        let stderr = receiver.try_recv().expect("stderr event");
        let end = receiver.try_recv().expect("end event");

        match begin.payload {
            EventPayload::ExecCommandBegin {
                turn_id: got_turn_id,
                exec_id: got_exec_id,
                command,
                cwd,
            } => {
                assert_eq!(got_turn_id, turn_id);
                assert_eq!(got_exec_id, exec_id);
                assert_eq!(command, vec!["echo".to_string(), "hi".to_string()]);
                assert_eq!(cwd, Some("/workspace/demo".to_string()));
            }
            other => panic!("unexpected begin payload: {other:?}"),
        }

        match stdout.payload {
            EventPayload::ExecCommandOutputDelta {
                turn_id: got_turn_id,
                exec_id: got_exec_id,
                stream,
                delta,
            } => {
                assert_eq!(got_turn_id, turn_id);
                assert_eq!(got_exec_id, exec_id);
                assert!(matches!(stream, ExecStream::Stdout));
                assert_eq!(delta, "out");
            }
            other => panic!("unexpected stdout payload: {other:?}"),
        }

        match stderr.payload {
            EventPayload::ExecCommandOutputDelta {
                turn_id: got_turn_id,
                exec_id: got_exec_id,
                stream,
                delta,
            } => {
                assert_eq!(got_turn_id, turn_id);
                assert_eq!(got_exec_id, exec_id);
                assert!(matches!(stream, ExecStream::Stderr));
                assert_eq!(delta, "err");
            }
            other => panic!("unexpected stderr payload: {other:?}"),
        }

        match end.payload {
            EventPayload::ExecCommandEnd {
                turn_id: got_turn_id,
                exec_id: got_exec_id,
                exit_code,
            } => {
                assert_eq!(got_turn_id, turn_id);
                assert_eq!(got_exec_id, exec_id);
                assert_eq!(exit_code, 7);
            }
            other => panic!("unexpected end payload: {other:?}"),
        }

        assert_eq!(begin.session_id, session_id);
    }

    #[tokio::test]
    async fn approval_handler_routes_requests_through_store() {
        let approvals = ApprovalStore::new();
        let (sender, mut receiver) = broadcast::channel(8);
        let session_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let handler = RuntimeApprovalHandler {
            session_id,
            turn_id,
            sender: sender.clone(),
            approvals: approvals.clone(),
        };

        let waiter = tokio::spawn(async move { handler.request_tool_approval("Read").await });

        let event = receiver.recv().await.expect("permission event");
        let request_id = match event.payload {
            EventPayload::PermissionRequested {
                turn_id: got_turn_id,
                request_id,
                request,
                ..
            } => {
                assert_eq!(got_turn_id, turn_id);
                assert_eq!(
                    format!("{request:?}"),
                    "Tool { name: \"Read\" }".to_string()
                );
                request_id
            }
            other => panic!("unexpected payload: {other:?}"),
        };

        assert!(approvals.resolve(request_id, ApprovalDecision::AllowOnce, sender));
        assert!(waiter.await.expect("join").is_ok());
    }
}
