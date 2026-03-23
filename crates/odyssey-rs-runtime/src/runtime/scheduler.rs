use super::engine::{OdysseyRuntimeInner, RunOutput};
use crate::{RuntimeError, runtime::executor::ScheduleExecutor};
use log::{error, info};
use odyssey_rs_protocol::{ExecutionHandle, ExecutionRequest, ExecutionStatus};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc, oneshot};
use uuid::Uuid;

struct ScheduledExecution {
    request: ExecutionRequest,
    handle: ExecutionHandle,
    completion: oneshot::Sender<Result<RunOutput, RuntimeError>>,
}

#[derive(Clone)]
pub(crate) struct ExecutionScheduler {
    sender: mpsc::Sender<ScheduledExecution>,
    statuses: Arc<RwLock<HashMap<Uuid, ExecutionStatus>>>,
}

impl ExecutionScheduler {
    pub(crate) fn new(
        inner: Arc<OdysseyRuntimeInner>,
        worker_count: usize,
        queue_capacity: usize,
    ) -> Self {
        let (sender, receiver) = mpsc::channel::<ScheduledExecution>(queue_capacity.max(1));
        let statuses = Arc::new(RwLock::new(HashMap::new()));
        let semaphore = Arc::new(Semaphore::new(worker_count.max(1)));
        let dispatcher_statuses = statuses.clone();

        spawn_dispatcher(receiver, semaphore, inner, dispatcher_statuses);

        info!("Execution Scheduler Initiated");

        Self { sender, statuses }
    }

    pub(crate) async fn submit(
        &self,
        request: ExecutionRequest,
    ) -> Result<
        (
            ExecutionHandle,
            oneshot::Receiver<Result<RunOutput, RuntimeError>>,
        ),
        RuntimeError,
    > {
        info!(
            "Execution Scheduler received request ID: {}",
            request.request_id
        );

        let handle = ExecutionHandle {
            session_id: request.session_id,
            turn_id: Uuid::new_v4(),
        };
        {
            let mut statuses = self.statuses.write();
            statuses.insert(handle.turn_id, ExecutionStatus::Queued);
        }
        let (completion_tx, completion_rx) = oneshot::channel();
        self.sender
            .send(ScheduledExecution {
                request,
                handle: handle.clone(),
                completion: completion_tx,
            })
            .await
            .map_err(|_| RuntimeError::Executor("execution scheduler stopped".to_string()))?;
        Ok((handle, completion_rx))
    }

    pub(crate) fn status(&self, turn_id: Uuid) -> Option<ExecutionStatus> {
        self.statuses.read().get(&turn_id).copied()
    }
}

fn spawn_dispatcher(
    mut receiver: mpsc::Receiver<ScheduledExecution>,
    semaphore: Arc<Semaphore>,
    inner: Arc<OdysseyRuntimeInner>,
    statuses: Arc<RwLock<HashMap<Uuid, ExecutionStatus>>>,
) {
    let dispatcher = async move {
        while let Some(job) = receiver.recv().await {
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => break,
            };
            let runtime = inner.clone();
            let statuses = statuses.clone();
            tokio::spawn(async move {
                let _permit = permit;
                {
                    let mut lock = statuses.write();
                    lock.insert(job.handle.turn_id, ExecutionStatus::Running);
                }
                info!("Execution for Request: {} starting", job.request.request_id);
                let exector = ScheduleExecutor::new(runtime);
                let result = exector
                    .execute_request(job.handle.turn_id, job.request.clone())
                    .await;
                {
                    let mut lock = statuses.write();
                    lock.insert(
                        job.handle.turn_id,
                        if result.is_ok() {
                            info!(
                                "Execution for Request: {} completed",
                                job.request.request_id
                            );

                            ExecutionStatus::Completed
                        } else {
                            error!("Execution for Request: {} failed", job.request.request_id);
                            ExecutionStatus::Failed
                        },
                    );
                }
                let _ = job.completion.send(result);
            });
        }
    };

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(dispatcher);
        }
        Err(_) => {
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build scheduler runtime");
                runtime.block_on(dispatcher);
            });
        }
    }
}
