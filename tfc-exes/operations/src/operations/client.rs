use futures::stream::StreamExt;
use log::{info, trace};
use std::cell::RefCell;
use std::sync::Arc;
use tokio::sync::{watch, Notify};
use tokio::task::JoinHandle;
use zbus::proxy;

use crate::operations::common::{OperationMode, OperationsUpdate};

#[proxy(
    interface = "is.centroid.OperationMode" /*DBUS_INTERFACE*/,
    default_service = "is.centroid.operations.def",
    default_path = "/is/centroid/OperationMode" /*DBUS_PATH*/,
)]
pub trait OperationsDBusClient {
    #[zbus(property)]
    fn mode(&self) -> zbus::fdo::Result<String>;
    #[zbus(signal)]
    fn update(&self, new_mode: &str, old_mode: &str) -> zbus::fdo::Result<()>;
    async fn set_mode(&self, mode_str: &str) -> zbus::fdo::Result<()>;
    async fn stop_with_reason(&self, reason: &str) -> zbus::fdo::Result<()>;
}

pub struct OperationsClient {
    #[allow(dead_code)]
    update_sender: watch::Sender<OperationsUpdate>,
    #[allow(dead_code)]
    set_mode_sender: watch::Sender<String>,
    #[allow(dead_code)]
    stop_sender: watch::Sender<String>,
    handles: RefCell<Vec<JoinHandle<()>>>, // I want non mutable self in the implementation
    #[allow(dead_code)]
    log_key: String,
}

impl OperationsClient {
    #[allow(dead_code)]
    pub fn new(bus: zbus::Connection) -> Self {
        let (update_sender, _) = watch::channel(OperationsUpdate::default());
        let (set_mode_sender, mut set_mode_receiver) = watch::channel(String::default());
        let (stop_sender, mut stop_receiver) = watch::channel(String::default());
        let log_key = "OperationsClient".to_string();
        let log_key_cp = log_key.clone();

        let update_sender_cp = update_sender.clone();
        let handle = tokio::spawn(async move {
            let proxy = OperationsDBusClientProxy::builder(&bus)
                .cache_properties(zbus::CacheProperties::No)
                .build()
                .await
                .expect("Failed to create OperationsDBusClientProxy");
            let mut update_receiver = proxy
                .receive_update()
                .await
                .expect("Failed to receive update");
            loop {
                tokio::select! {
                    _ = set_mode_receiver.changed() => {
                        let mode = set_mode_receiver.borrow_and_update().clone();
                        trace!(target: &log_key_cp, "Proxying set mode to {:?}", mode);
                        proxy.set_mode(&mode).await.expect("Failed to set mode");
                    }
                    _ = stop_receiver.changed() => {
                        let reason = stop_receiver.borrow_and_update().clone();
                        trace!(target: &log_key_cp, "Proxying stop with reason {:?}", reason);
                        proxy.stop_with_reason(&reason).await.expect("Failed to send stop");
                    }
                    Some(update) = update_receiver.next() => {
                        let args = update.args().expect("Failed to get update args");
                        let new_mode: OperationMode = args.new_mode().parse().expect("Failed to parse new mode");
                        let old_mode: OperationMode = args.old_mode().parse().expect("Failed to parse old mode");
                        if new_mode == old_mode || new_mode == OperationMode::Unknown {
                            info!(target: &log_key_cp, "Ignoring update: new_mode={:?}, old_mode={:?}", new_mode, old_mode);
                            continue;
                        }
                        trace!(target: &log_key_cp, "Proxying update: new_mode={:?}, old_mode={:?}", new_mode, old_mode);
                        update_sender_cp.send(OperationsUpdate {
                            new_mode,
                            old_mode,
                        }).expect("Failed to send update");
                    }
                }
            }
        });
        Self {
            update_sender,
            set_mode_sender,
            stop_sender,
            handles: RefCell::new(vec![handle]),
            log_key,
        }
    }
    #[allow(dead_code)]
    pub fn set_mode(&self, mode: OperationMode) {
        trace!(target: &self.log_key, "Sending set mode to {:?}", mode);
        self.set_mode_sender
            .send(mode.to_string())
            .expect("Failed to send mode");
    }
    #[allow(dead_code)]
    pub fn stop(&self, reason: &str) {
        self.stop_sender
            .send(reason.to_string())
            .expect("Failed to send stop");
    }
    #[allow(dead_code)]
    pub fn subscribe_updates(&self) -> watch::Receiver<OperationsUpdate> {
        self.update_sender.subscribe()
    }
    #[allow(dead_code)]
    pub fn subscribe_entry_mode(&self, operation_mode: OperationMode) -> Arc<Notify> {
        let mut update_receiver = self.subscribe_updates();
        let notify = Arc::new(Notify::new());
        let notify_cp = notify.clone();
        let log_key = self.log_key.clone();
        self.handles.borrow_mut().push(tokio::spawn(async move {
            trace!(target: &log_key, "Subscribed to entry of {:?}", operation_mode);
            while let Ok(_) = update_receiver.changed().await {
                let update = update_receiver.borrow_and_update();
                if update.new_mode == operation_mode {
                    notify.notify_waiters();
                }
            }
        }));
        notify_cp
    }
    #[allow(dead_code)]
    pub fn subscribe_exit_mode(&self, operation_mode: OperationMode) -> Arc<Notify> {
        let mut update_receiver = self.subscribe_updates();
        let notify = Arc::new(Notify::new());
        let notify_cp = notify.clone();
        let log_key = self.log_key.clone();
        self.handles.borrow_mut().push(tokio::spawn(async move {
            trace!(target: &log_key, "Subscribed to exit of {:?}", operation_mode);
            while let Ok(_) = update_receiver.changed().await {
                let update = update_receiver.borrow_and_update();
                if update.old_mode == operation_mode {
                    notify.notify_waiters();
                }
            }
        }));
        notify_cp
    }
    #[allow(dead_code)]
    pub fn mode(&self) -> OperationMode {
        self.update_sender.borrow().new_mode
    }
}

impl Drop for OperationsClient {
    fn drop(&mut self) {
        for handle in self.handles.borrow_mut().drain(..) {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::server::{OperationsImpl, OperationsStateMachine};
    use tokio::sync::Mutex;

    struct TestContext {
        _bus: zbus::Connection,
        client: OperationsClient,
        _server: Arc<Mutex<OperationsStateMachine<OperationsImpl>>>,
    }

    impl TestContext {
        async fn new() -> Result<Self, Box<dyn std::error::Error>> {
            // tfc::progbase::init();
            tfc::logger::init_test_logger(log::LevelFilter::Trace)?;
            let _bus = zbus::connection::Builder::system()?
                .name("is.centroid.operations.def")?
                .build()
                .await?;
            let client = OperationsClient::new(_bus.clone());
            let (_server, _) = OperationsImpl::spawn(_bus.clone());
            Ok(Self {
                _bus,
                client,
                _server,
            })
        }
    }

    #[tokio::test]
    async fn test_running_and_stopped() {
        let ctx = TestContext::new()
            .await
            .expect("Failed to create test context");

        // Test entering running
        let running_notify = ctx.client.subscribe_entry_mode(OperationMode::Running);
        let task = async {
            ctx.client.set_mode(OperationMode::Running);
        };
        tokio::join!(running_notify.notified(), task);
        assert_eq!(ctx.client.mode(), OperationMode::Running);

        // Test exiting running and entering stopped
        let running_exit_notify = ctx.client.subscribe_exit_mode(OperationMode::Running);
        let stopped_notify = ctx.client.subscribe_entry_mode(OperationMode::Stopped);
        let task = async {
            ctx.client.set_mode(OperationMode::Stopped);
        };
        tokio::join!(
            running_exit_notify.notified(),
            stopped_notify.notified(),
            task
        );
        assert_eq!(ctx.client.mode(), OperationMode::Stopped);
    }
}
