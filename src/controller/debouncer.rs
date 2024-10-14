use super::{appraiser::Ctx, CargoDocumentEvent};
use std::{pin::Pin, time::Duration};
use tokio::sync::mpsc::{self, error::SendError, Sender};
use tokio::time::{sleep, Sleep};
use tower_lsp::lsp_types::Url;

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
        let interactive_timeout = self.interactive_timeout;
        let background_timeout = self.background_timeout;

        tokio::spawn(async move {
            let mut uri: Option<Url> = None;
            let mut rev = 0;
            let mut delay: Option<Pin<Box<Sleep>>> = None;
            let mut backoff_uri: Option<Url> = None;
            let mut backoff_factor: u32 = 0;

            loop {
                tokio::select! {
                    // Handle incoming Ctx messages
                    some_event = internal_rx.recv() => {
                        let Some(event) = some_event else{break};
                        let timeout=match event {
                              DebouncerEvent::Interactive(ctx) => {
                                    uri = Some(ctx.uri.clone());
                                    rev = ctx.rev;
                                    //reset backoff
                                    backoff_uri = Some(ctx.uri.clone());
                                    backoff_factor = 0;
                                    interactive_timeout
                                },
                                DebouncerEvent::Background(ctx) => {
                                    if let Some(uri) = &backoff_uri {
                                        if uri == &ctx.uri {
                                            backoff_factor += 1;
                                        } else {
                                            backoff_uri = Some(ctx.uri.clone());
                                            backoff_factor = 0;
                                        }
                                    }
                                    uri = Some(ctx.uri.clone());
                                    rev = ctx.rev;
                                    calculate_backoff_timeout(background_timeout, backoff_factor)
                                }
                        };
                        delay = Some(Box::pin(sleep(Duration::from_millis(timeout))));
                    }

                    // Handle the delay if it's set
                    () = async {
                        if let Some(ref mut d) = delay {
                            d.await
                        } else {
                            futures::future::pending::<()>().await
                        }
                    }, if delay.is_some() => {
                        if let Some(current_uri) = &uri {
                            let ctx = Ctx {
                                uri: current_uri.clone(),
                                rev,
                            };
                            if let Err(e) = tx.send(CargoDocumentEvent::ReadyToResolve(ctx)).await {
                                eprintln!("Failed to send Ctx: {}", e);
                            }
                        }
                        // Reset the delay
                        delay = None;
                    }
                }
            }
        });
    }
}

fn calculate_backoff_timeout(base_timeout: u64, count: u32) -> u64 {
    let factor = match count {
        0..=5 => 1,
        6..=10 => 2,
        11..=15 => 3,
        16..=20 => 4,
        _ => 5,
    };
    (base_timeout * factor).min(15_000) // Cap at 15 seconds
}
