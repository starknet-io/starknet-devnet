//! Ordered, bounded socket output. Enqueuing never waits for network I/O, so a slow client
//! cannot hold the lifecycle lock or prevent other subscriptions from receiving updates.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::Message;
use futures::{Sink, SinkExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};

// Allow the largest supported new-heads history (1025 headers) plus confirmation to be queued
// in one uninterrupted task poll, even on a single-thread runtime.
const MAX_QUEUED_MESSAGES: usize = 4096;
const MAX_QUEUED_BYTES: u32 = 64 * 1024 * 1024;
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

struct QueuedMessage {
    text: String,
    budget: OwnedSemaphorePermit,
}

#[derive(Clone)]
pub(crate) struct SocketSender {
    sender: mpsc::Sender<QueuedMessage>,
    budget: Arc<Semaphore>,
    closed: watch::Sender<bool>,
}

pub(crate) struct SocketWriter {
    receiver: mpsc::Receiver<QueuedMessage>,
    closed: watch::Sender<bool>,
}

impl SocketSender {
    pub(crate) fn channel() -> (Self, SocketWriter) {
        let (sender, receiver) = mpsc::channel(MAX_QUEUED_MESSAGES);
        let (closed, _) = watch::channel(false);
        (
            Self {
                sender,
                budget: Arc::new(Semaphore::new(MAX_QUEUED_BYTES as usize)),
                closed: closed.clone(),
            },
            SocketWriter { receiver, closed },
        )
    }

    pub(crate) fn closed(&self) -> watch::Receiver<bool> {
        self.closed.subscribe()
    }

    pub(crate) fn send(&self, text: String) {
        if *self.closed.borrow() {
            return;
        }
        let budget = u32::try_from(text.len())
            .ok()
            .and_then(|len| self.budget.clone().try_acquire_many_owned(len).ok());
        let Some(budget) = budget else {
            self.disconnect();
            return;
        };
        if self.sender.try_send(QueuedMessage { text, budget }).is_err() {
            self.disconnect();
        }
    }

    fn disconnect(&self) {
        tracing::warn!("Disconnecting websocket: output queue is full or closed");
        self.closed.send_replace(true);
    }
}

impl SocketWriter {
    pub(crate) async fn run<S>(mut self, mut sink: S)
    where
        S: Sink<Message> + Unpin,
        S::Error: std::fmt::Display,
    {
        let mut closed = self.closed.subscribe();
        tokio::select! {
            biased;
            _ = closed.wait_for(|value| *value) => {},
            _ = async {
                while let Some(message) = self.receiver.recv().await {
                    match tokio::time::timeout(
                        SEND_TIMEOUT, sink.send(Message::Text(message.text.into())),
                    ).await {
                        Ok(Ok(())) => {},
                        Ok(Err(error)) => {
                            tracing::warn!(%error, "Websocket write failed");
                            break;
                        }
                        Err(_) => {
                            tracing::warn!("Disconnecting websocket: write timed out");
                            break;
                        }
                    }
                    drop(message.budget);
                }
            } => {},
        }
        self.closed.send_replace(true);
        // Give responsive clients a close handshake without delaying state-changing requests.
        let _ = tokio::time::timeout(Duration::from_secs(1), sink.close()).await;
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::{MAX_QUEUED_BYTES, MAX_QUEUED_MESSAGES, SocketSender};

    #[tokio::test]
    async fn output_preserves_response_and_notification_order() {
        let (sender, writer) = SocketSender::channel();
        let (output, mut received) = futures::channel::mpsc::unbounded();
        for text in ["confirmation", "reorg", "restored head", "RPC result"] {
            sender.send(text.into());
        }
        drop(sender);
        writer.run(output).await;
        let mut texts = Vec::new();
        while let Some(message) = received.next().await {
            texts.push(message.into_text().unwrap().to_string());
        }
        assert_eq!(texts, ["confirmation", "reorg", "restored head", "RPC result"]);
    }

    #[tokio::test]
    async fn saturated_client_does_not_block_another_client() {
        let (slow, mut slow_writer) = SocketSender::channel();
        let (healthy, mut healthy_writer) = SocketSender::channel();
        for _ in 0..MAX_QUEUED_MESSAGES {
            slow.send("queued".into());
        }
        assert!(!*slow.closed().borrow());
        slow.send("overflow".into());
        assert!(*slow.closed().borrow());
        healthy.send("reorg".into());
        assert!(!*healthy.closed().borrow());
        assert_eq!(healthy_writer.receiver.try_recv().unwrap().text, "reorg");
        // Queue saturation disconnects instead of silently dropping one notification and
        // continuing to deliver an inconsistent stream to that client.
        assert_eq!(slow_writer.receiver.try_recv().unwrap().text, "queued");
    }

    #[tokio::test]
    async fn output_budget_also_limits_bytes() {
        let (sender, writer) = SocketSender::channel();
        let reserved = sender.budget.clone().acquire_many_owned(MAX_QUEUED_BYTES).await.unwrap();
        sender.send("no capacity left".into());
        assert!(*sender.closed().borrow());
        drop(reserved);
        drop(writer);
    }
}
