use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::StreamExt;
use odyssey_rs_runtime::RuntimeEvent;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

fn event_stream(
    receiver: broadcast::Receiver<RuntimeEvent>,
) -> impl futures_util::Stream<Item = Result<Event, axum::Error>> {
    BroadcastStream::new(receiver).filter_map(|message| async move {
        match message {
            Ok(event) => match serde_json::to_string(&event) {
                Ok(json) => Some(Ok(Event::default().data(json))),
                Err(_) => None,
            },
            Err(_) => None,
        }
    })
}

pub fn stream_events(
    receiver: broadcast::Receiver<RuntimeEvent>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, axum::Error>>> {
    Sse::new(event_stream(receiver)).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::event_stream;
    use futures_util::StreamExt;
    use odyssey_rs_protocol::EventPayload;
    use odyssey_rs_runtime::RuntimeEvent;
    use pretty_assertions::assert_eq;
    use tokio::sync::broadcast;
    use uuid::Uuid;

    #[tokio::test]
    async fn stream_events_serializes_runtime_events() {
        let (sender, receiver) = broadcast::channel(4);
        let event = RuntimeEvent {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            created_at: chrono::Utc::now(),
            payload: EventPayload::ReasoningSectionBreak {
                turn_id: Uuid::new_v4(),
            },
        };
        sender.send(event.clone()).expect("send event");

        let mut stream = Box::pin(event_stream(receiver));
        let rendered = stream.next().await.expect("sse item");
        assert_eq!(rendered.is_ok(), true);
    }
}
