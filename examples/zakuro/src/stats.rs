use std::time::Duration;

use shiguredo_webrtc::{rtc_log_info, rtc_log_warning};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

pub(crate) enum StatsEvent {
    Connected { id: u32 },
    Disconnected { id: u32 },
    Retrying { id: u32, retry_count: u32 },
    Stopped { id: u32 },
}

#[derive(Clone)]
pub(crate) struct StatsSnapshot {
    pub(crate) total: u32,
    pub(crate) connected: u32,
    pub(crate) retrying: u32,
    pub(crate) stopped: u32,
}

impl StatsSnapshot {
    fn initial(total: u32) -> Self {
        Self {
            total,
            connected: 0,
            retrying: 0,
            stopped: 0,
        }
    }

    fn apply(&mut self, event: StatsEvent) {
        match event {
            StatsEvent::Connected { id } => {
                rtc_log_info!("[stats] vc-{} connected", id);
                self.connected += 1;
                if self.retrying > 0 {
                    self.retrying -= 1;
                }
            }
            StatsEvent::Disconnected { id } => {
                rtc_log_info!("[stats] vc-{} disconnected", id);
                if self.connected > 0 {
                    self.connected -= 1;
                }
            }
            StatsEvent::Retrying { id, retry_count } => {
                rtc_log_warning!("[stats] vc-{} retrying ({})", id, retry_count);
                self.retrying += 1;
            }
            StatsEvent::Stopped { id } => {
                rtc_log_info!("[stats] vc-{} stopped", id);
                self.stopped += 1;
                if self.retrying > 0 {
                    self.retrying -= 1;
                }
            }
        }
    }
}

pub(crate) struct StatsCollector {
    event_tx: mpsc::Sender<StatsEvent>,
    _snapshot_rx: watch::Receiver<StatsSnapshot>,
}

impl StatsCollector {
    pub(crate) fn new(total: u32, token: CancellationToken) -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        let (snapshot_tx, snapshot_rx) = watch::channel(StatsSnapshot::initial(total));

        tokio::spawn(Self::aggregator(
            event_rx,
            snapshot_tx,
            total,
            token.clone(),
        ));
        tokio::spawn(Self::reporter(snapshot_rx.clone(), token));

        Self {
            event_tx,
            _snapshot_rx: snapshot_rx,
        }
    }

    pub(crate) fn event_tx(&self) -> mpsc::Sender<StatsEvent> {
        self.event_tx.clone()
    }

    async fn aggregator(
        mut event_rx: mpsc::Receiver<StatsEvent>,
        snapshot_tx: watch::Sender<StatsSnapshot>,
        total: u32,
        token: CancellationToken,
    ) {
        let mut snapshot = StatsSnapshot::initial(total);
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => break,
                event = event_rx.recv() => {
                    let Some(event) = event else { break };
                    snapshot.apply(event);
                    let _ = snapshot_tx.send(snapshot.clone());
                }
            }
        }
    }

    async fn reporter(mut snapshot_rx: watch::Receiver<StatsSnapshot>, token: CancellationToken) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => break,
                _ = interval.tick() => {
                    let snap = snapshot_rx.borrow_and_update().clone();
                    rtc_log_info!(
                        "[stats] total={} connected={} retrying={} stopped={}",
                        snap.total,
                        snap.connected,
                        snap.retrying,
                        snap.stopped,
                    );
                }
            }
        }
    }
}
