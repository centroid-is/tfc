use crate::operations::common::{OperationMode, OperationsUpdate};
use crate::operations::server::OperationsEvents;

use log::info;
use tokio::sync::watch;
use zbus::interface;

pub static DBUS_PATH: &str = "/is/centroid/OperationMode";
pub static _DBUS_INTERFACE: &str = "is.centroid.OperationMode";

pub struct OperationsDBusService {
    log_key: String,
    sm_event_sender: watch::Sender<OperationsEvents>,
    mode_update_receiver: watch::Receiver<OperationsUpdate>,
}

impl OperationsDBusService {
    pub fn new(
        sm_event_sender: watch::Sender<OperationsEvents>,
        mode_update_receiver: watch::Receiver<OperationsUpdate>,
        key: &str,
    ) -> Self {
        Self {
            log_key: key.to_string(),
            sm_event_sender,
            mode_update_receiver,
        }
    }
}
#[interface(name =  "is.centroid.OperationMode" /*DBUS_INTERFACE*/)]
impl OperationsDBusService {
    #[zbus(property)]
    async fn mode(&self) -> Result<String, zbus::fdo::Error> {
        Ok(self.mode_update_receiver.borrow().new_mode.to_string())
    }

    #[zbus(signal)]
    pub async fn update(
        signal_ctxt: &zbus::SignalContext<'_>,
        new_mode: &str,
        old_mode: &str,
    ) -> zbus::Result<()>;

    async fn set_mode(&mut self, mode_str: &str) -> Result<(), zbus::fdo::Error> {
        info!(target: &self.log_key, "Trying to set operation mode to {:?}", mode_str);
        let mode: OperationMode = mode_str.parse().map_or(OperationMode::Unknown, |mode| mode);
        let event = match mode {
            OperationMode::Unknown => {
                let err_msg = format!("Invalid operation mode: {}", mode_str);
                info!(target: &self.log_key, "{}", err_msg);
                return Err(zbus::fdo::Error::Failed(err_msg));
            }
            OperationMode::Stopped | OperationMode::Stopping => {
                OperationsEvents::SetStopped("manual".to_string())
            }
            OperationMode::Starting | OperationMode::Running => OperationsEvents::SetStarting,
            OperationMode::Cleaning => OperationsEvents::SetCleaning,
            OperationMode::Emergency => OperationsEvents::SetEmergency,
            OperationMode::Fault => OperationsEvents::SetFault,
            OperationMode::Maintenance => OperationsEvents::SetMaintenance,
        };
        self.sm_event_sender.send(event).map_err(|e| {
            let err_msg = format!("Error sending stop: {}", e);
            info!(target: &self.log_key, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })?;
        Ok(())
    }

    async fn stop_with_reason(&mut self, reason: &str) -> Result<(), zbus::fdo::Error> {
        self.sm_event_sender
            .send(OperationsEvents::SetStopped(reason.to_string()))
            .map_err(|e| {
                let err_msg = format!("Error sending stop: {}", e);
                info!(target: &self.log_key, "{}", err_msg);
                zbus::fdo::Error::Failed(err_msg)
            })?;

        Ok(())
    }
}
