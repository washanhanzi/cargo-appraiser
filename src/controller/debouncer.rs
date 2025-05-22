use crate::entity::CanonicalUri;

use super::{appraiser::Ctx, CargoDocumentEvent};
use futures::{Stream, StreamExt};
use std::{
    collections::HashMap,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc::{self, error::SendError, Sender};
use tokio_util::time::{delay_queue, DelayQueue};
use tracing::error;

// Change Timer
pub struct Debouncer {
    tx: Sender<CargoDocumentEvent>,
    interactive_timeout: u64,
    background_timeout: u64,
    sender: Option<Sender<DebouncerEvent>>,
}

pub enum DebouncerEvent {
    Interactive(Ctx),
    Background(Ctx),
}

pub struct Queue {
    entries: HashMap<CanonicalUri, (usize, delay_queue::Key)>,
    expirations: DelayQueue<CanonicalUri>,
    backoff_factor: HashMap<CanonicalUri, u32>,
    background_timeout: u64,
    interactive_timeout: u64,
}

impl Queue {
    pub fn new(interactive_timeout: u64, background_timeout: u64) -> Self {
        Self {
            entries: HashMap::new(),
            expirations: DelayQueue::new(),
            backoff_factor: HashMap::new(),
            background_timeout,
            interactive_timeout,
        }
    }

    pub fn insert_interactive(&mut self, ctx: Ctx) {
        self.backoff_factor.remove(&ctx.uri);
        let key = self.expirations.insert(
            ctx.uri.clone(),
            Duration::from_millis(self.interactive_timeout),
        );
        self.entries.insert(ctx.uri, (ctx.rev, key));
    }

    pub fn insert_background(&mut self, ctx: Ctx) {
        let factor = self.backoff_factor.entry(ctx.uri.clone()).or_insert(0);
        *factor += 1;
        let timeout = calculate_backoff_timeout(self.background_timeout, *factor);
        let key = self
            .expirations
            .insert(ctx.uri.clone(), Duration::from_millis(timeout));
        self.entries.insert(ctx.uri, (ctx.rev, key));
    }
}

impl Stream for Queue {
    type Item = Ctx;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        while let Poll::Ready(Some(expired)) = this.expirations.poll_expired(cx) {
            if let Some((rev, _)) = this.entries.remove(expired.get_ref()) {
                return Poll::Ready(Some(Ctx {
                    uri: expired.get_ref().clone(),
                    rev,
                }));
            } // If not found in entries, just loop to the next expired item.
        }

        // Only return None when the DelayQueue itself is empty.
        if this.expirations.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

impl Debouncer {
    pub fn new(
        tx: Sender<CargoDocumentEvent>,
        interactive_timeout: u64,
        background_timeout: u64,
    ) -> Self {
        Self {
            tx,
            interactive_timeout,
            background_timeout,
            sender: None,
        }
    }

    pub async fn send_interactive(&self, ctx: Ctx) -> Result<(), SendError<DebouncerEvent>> {
        self.sender
            .as_ref()
            .unwrap()
            .send(DebouncerEvent::Interactive(ctx))
            .await
    }

    pub async fn send_background(&self, ctx: Ctx) -> Result<(), SendError<DebouncerEvent>> {
        self.sender
            .as_ref()
            .unwrap()
            .send(DebouncerEvent::Background(ctx))
            .await
    }

    pub fn spawn(&mut self) {
        // Create a tokio mpsc channel
        let (internal_tx, mut internal_rx) = mpsc::channel::<DebouncerEvent>(64);
        self.sender = Some(internal_tx);
        let tx = self.tx.clone();
        let mut q = Queue::new(self.interactive_timeout, self.background_timeout);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Handle incoming Ctx messages
                    Some(event) = internal_rx.recv() => {
                        match event {
                            DebouncerEvent::Interactive(ctx) => {
                                q.insert_interactive(ctx);
                            },
                            DebouncerEvent::Background(ctx) => {
                                q.insert_background(ctx);
                            }
                        };
                    }
                    Some(ctx) = q.next() => {
                        if let Err(e) = tx.send(CargoDocumentEvent::ReadyToResolve(ctx)).await {
                            error!("failed to send Ctx from debouncer: {}", e);
                        }
                    }
                }
            }
        });
    }
}

fn calculate_backoff_timeout(base_timeout: u64, count: u32) -> u64 {
    let factor = match count {
        0..=2 => 1,
        3..=5 => 2,
        6..=10 => 3,
        11..=15 => 6,
        _ => 7,
    };
    (base_timeout * factor).min(30_000) // Cap at 30 seconds
}
