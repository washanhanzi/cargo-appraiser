use std::time::Duration;

use tokio::sync::mpsc::{self, Sender};
use tokio::time::{sleep, Instant};
use tower_lsp::lsp_types::Url;

use super::{appraiser::Ctx, CargoDocumentEvent};

//change timer
pub struct ChangeTimer {
    tx: Sender<CargoDocumentEvent>,
    timeout: u64,
}

impl ChangeTimer {
    pub fn new(tx: Sender<CargoDocumentEvent>, timeout: u64) -> Self {
        Self { tx, timeout }
    }

    pub fn spawn(&self) -> Sender<Ctx> {
        //create a tokio mpsc channel
        let (internal_tx, mut internal_rx) = mpsc::channel::<Ctx>(32);
        let tx = self.tx.clone();
        let timeout = self.timeout;
        tokio::spawn(async move {
            let mut uri: Option<Url> = None;
            let mut rev = 0;
            let delay = sleep(Duration::from_millis(timeout));
            tokio::pin!(delay);
            loop {
                tokio::select! {
                    Some(ctx) = internal_rx.recv() => {
                        uri = Some(ctx.uri.clone());
                        rev = ctx.rev;
                        // Reset the delay to 1 second after receiving a message
                        delay.as_mut().reset(Instant::now() + Duration::from_millis(timeout));
                    }
                    _ = &mut delay => {
                        if let Some(current_uri) = &uri {
                            let ctx = Ctx {
                                uri: current_uri.clone(),
                                rev,
                            };
                            if let Err(e) = tx.send(CargoDocumentEvent::ChangeTimer(ctx)).await {
                                eprintln!("Failed to send Ctx: {}", e);
                            }
                        }
                        // Reset the delay after it has elapsed
                        delay.as_mut().reset(Instant::now() + Duration::from_millis(timeout));
                    }
                }
            }
        });
        internal_tx
    }
}
