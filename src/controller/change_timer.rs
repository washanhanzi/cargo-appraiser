use super::{appraiser::Ctx, CargoDocumentEvent};
use std::{pin::Pin, time::Duration};
use tokio::sync::mpsc::{self, Sender};
use tokio::time::{sleep, Instant, Sleep};
use tower_lsp::lsp_types::Url;

// Change Timer
pub struct ChangeTimer {
    tx: Sender<CargoDocumentEvent>,
    timeout: u64,
}

impl ChangeTimer {
    pub fn new(tx: Sender<CargoDocumentEvent>, timeout: u64) -> Self {
        Self { tx, timeout }
    }

    pub fn spawn(&self) -> Sender<Ctx> {
        // Create a tokio mpsc channel
        let (internal_tx, mut internal_rx) = mpsc::channel::<Ctx>(32);
        let tx = self.tx.clone();
        let timeout = self.timeout;

        tokio::spawn(async move {
            let mut uri: Option<Url> = None;
            let mut rev = 0;
            let mut delay: Option<Pin<Box<Sleep>>> = None;

            loop {
                tokio::select! {
                    // Handle incoming Ctx messages
                    some_ctx = internal_rx.recv() => {
                        if let Some(ctx) = some_ctx {
                            uri = Some(ctx.uri.clone());
                            rev = ctx.rev;

                            // Initialize or reset the delay
                            delay = Some(Box::pin(sleep(Duration::from_millis(timeout))));
                        } else {
                            // Channel closed, exit the loop
                            break;
                        }
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
                            if let Err(e) = tx.send(CargoDocumentEvent::ChangeTimer(ctx)).await {
                                eprintln!("Failed to send Ctx: {}", e);
                            }
                        }
                        // Reset the delay
                        delay = None;
                    }
                }
            }
        });

        internal_tx
    }
}
