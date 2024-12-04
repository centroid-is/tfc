use derive_more::Display;
use log::{info, trace, warn};
#[cfg(feature = "opcua-expose")]
use opcua::server::{
    node_manager::memory::{InMemoryNodeManager, SimpleNodeManagerImpl},
    SubscriptionCache,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use smlang::statemachine;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use tfc::confman;
use tfc::ipc::{Base, Signal, Slot};
use tfc::time::MilliDuration;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::operations::client::OperationsClient;
use crate::operations::common::{OperationMode, OperationsUpdate};
// Define the state machine transitions and actions
statemachine! {
    name: Operations,
    derive_states: [Debug, Display, Clone],
    derive_events: [Debug, Display, Clone],
    transitions: {
        *Init + SetStopped(String) = Stopped,

        Stopped + SetStarting = Starting,
        Stopped + RunButton = Starting,
        Starting + StartingTimeout = Running,
        Starting + StartingFinished = Running,
        Starting + RunButton = Stopped, // todo discuss should this be to stopping?
        Starting + SetStopped(String) = Stopped, // todo discuss should this be to stopping?
        Running + RunButton = Stopping,
        Running + SetStopped(String) = Stopping,
        Stopping + StoppingTimeout = Stopped,
        Stopping + StoppingFinished = Stopped,

        Stopped + CleaningButton = Cleaning,
        Stopped + SetCleaning = Cleaning,
        Cleaning + CleaningButton = Stopped,
        Cleaning + SetStopped(String) = Stopped,

        // Transitions to Emergency
        _ + SetEmergency = Emergency,

        _ + EmergencyOn = Emergency,

        Emergency + EmergencyOff [!is_fault] = Stopped,
        Emergency + EmergencyOff [is_fault] = Fault,

        Stopped + FaultOn = Fault,
        Stopped + SetFault = Fault,
        Running + FaultOn = Fault,
        Running + SetFault = Fault,

        Fault + FaultOff = Stopped,

        Stopped + MaintenanceButton = Maintenance,
        Stopped + SetMaintenance = Maintenance,
        Maintenance + MaintenanceButton = Stopped,
        Maintenance + SetStopped(String) = Stopped,
    },
}

impl From<&OperationsStates> for OperationMode {
    fn from(state: &OperationsStates) -> Self {
        match state {
            OperationsStates::Stopped => OperationMode::Stopped,
            OperationsStates::Starting => OperationMode::Starting,
            OperationsStates::Running => OperationMode::Running,
            OperationsStates::Stopping => OperationMode::Stopping,
            OperationsStates::Cleaning => OperationMode::Cleaning,
            OperationsStates::Emergency => OperationMode::Emergency,
            OperationsStates::Fault => OperationMode::Fault,
            OperationsStates::Maintenance => OperationMode::Maintenance,
            OperationsStates::Init => OperationMode::Unknown,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct Storage {
    #[schemars(
        description = "Delay to run initial sequences to get the equipment ready. Milliseconds"
    )]
    startup_time: Option<MilliDuration>,
    #[schemars(
        description = "Delay to run shutdown sequences to get the equipment ready for being stopped. Milliseconds"
    )]
    stopping_time: Option<MilliDuration>,
}

impl Default for Storage {
    fn default() -> Self {
        Self {
            startup_time: Some(Duration::from_millis(0).into()),
            stopping_time: Some(Duration::from_millis(0).into()),
        }
    }
}

#[cfg(feature = "opcua-expose")]
pub trait OpcuaExpose {
    fn opcua_expose(
        &self,
        manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
        subscriptions: Arc<SubscriptionCache>,
        namespace: u16,
    );
}

#[cfg(feature = "opcua-expose")]
impl OpcuaExpose for OperationsStateMachine<OperationsImpl> {
    fn opcua_expose(
        &self,
        manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
        subscriptions: Arc<SubscriptionCache>,
        namespace: u16,
    ) {
        self.context.opcua_expose(manager, subscriptions, namespace);
    }
}

pub struct OperationsImpl {
    log_key: String,
    stopped: Signal<bool>,
    starting: Signal<bool>,
    running: Signal<bool>,
    stopping: Signal<bool>,
    cleaning: Signal<bool>,
    emergency_out: Signal<bool>,
    fault_out: Signal<bool>,
    maintenance_out: Signal<bool>,
    mode: Signal<u64>,
    mode_str: Signal<String>,
    stop_reason: Signal<String>,
    starting_finished: Slot<bool>,
    stopping_finished: Slot<bool>,
    run_button: Slot<bool>,
    cleaning_button: Slot<bool>,
    maintenance_button: Slot<bool>,
    emergency_in: Slot<bool>,
    fault_in: Slot<bool>,
    confman: confman::ConfMan<Storage>,
    sm_event_sender: watch::Sender<OperationsEvents>,
    mode_update_sender: watch::Sender<OperationsUpdate>,
    starting_handle: Option<JoinHandle<()>>,
    stopping_handle: Option<JoinHandle<()>>,
    bus: zbus::Connection,
}

impl OperationsImpl {
    pub fn spawn(
        bus: zbus::Connection,
    ) -> (
        Arc<Mutex<OperationsStateMachine<OperationsImpl>>>,
        JoinHandle<()>,
    ) {
        let (mode_update_tx, mut mode_update_rx) = watch::channel(OperationsUpdate {
            old_mode: OperationMode::Unknown,
            new_mode: OperationMode::Unknown,
        });
        let (sm_event_tx, mut sm_event_rx) =
            watch::channel(OperationsEvents::SetStopped("init".to_string()));

        let context = OperationsImpl::new(bus.clone(), mode_update_tx.clone(), sm_event_tx.clone());
        let statemachine = Arc::new(Mutex::new(OperationsStateMachine::new(context)));

        // Process events from the event channel
        let log_key = "Operations"; // this is copy but it's fine
        (
            statemachine.clone(),
            tokio::spawn(async move {
                // Create two async tasks
                let event_task = async {
                    while let Ok(_) = sm_event_rx.changed().await {
                        let event = sm_event_rx.borrow_and_update().clone();
                        let event_str = event.to_string();
                        let mut sm = statemachine.lock().await;
                        let res = sm
                        .process_event(event.clone())
                        .map_err(|e| match e {
                            OperationsError::ActionFailed(_action) => {
                                info!(target: &log_key, "During processEvent: {} Action failed", event_str)
                            }
                            OperationsError::GuardFailed(_guard) => {
                                info!(target: &log_key, "During processEvent: {} Guard failed", event_str);
                            }
                            OperationsError::InvalidEvent => {
                                info!(target: &log_key, "During processEvent: {} Invalid event", event_str);
                            }
                            OperationsError::TransitionsFailed => {
                                info!(target: &log_key, "During processEvent: {} Transitions failed", event_str);
                            }
                        });
                        if res.is_ok() {
                            match event {
                                OperationsEvents::SetStopped(reason) => {
                                    sm.context.update_stop_reason(&reason);
                                }
                                _ => {
                                    sm.context.update_stop_reason("");
                                }
                            }
                        }
                    }
                    log::error!("Event channel closed");
                };

                let dbus_task = async {
                    let client = OperationsClient::new(
                        sm_event_tx.clone(),
                        mode_update_tx.subscribe(),
                        "OperationMode",
                    );
                    let res = bus
                        .object_server()
                        .at(crate::operations::client::DBUS_PATH, client)
                        .await
                        .expect(&format!(
                            "Error registering object: {}",
                            crate::operations::client::DBUS_PATH
                        ));
                    if !res {
                        log::error!(
                            "Interface OperationMode already registered at {}",
                            crate::operations::client::DBUS_PATH
                        );
                    }
                    let iface: zbus::InterfaceRef<OperationsClient> = loop {
                        match bus
                            .object_server()
                            .interface(crate::operations::client::DBUS_PATH)
                            .await
                        {
                            Ok(iface) => break iface,
                            Err(_) => {
                                tokio::time::sleep(Duration::from_millis(1000)).await;
                                continue;
                            }
                        }
                    };
                    while let Ok(_) = mode_update_rx.changed().await {
                        let new_mode_str;
                        let old_mode_str;
                        {
                            let update = mode_update_rx.borrow_and_update();
                            new_mode_str = update.new_mode.to_string();
                            old_mode_str = update.old_mode.to_string();
                        }
                        let _ = OperationsClient::update(
                            &iface.signal_context(),
                            &new_mode_str,
                            &old_mode_str,
                        )
                        .await;
                    }
                    log::error!("Mode update channel closed");
                };

                // Run both tasks concurrently
                tokio::join!(event_task, dbus_task);
            }),
        )
    }

    pub fn new(
        bus: zbus::Connection,
        mode_update_sender: watch::Sender<OperationsUpdate>,
        sm_event_sender: watch::Sender<OperationsEvents>,
    ) -> Self {
        let log_key = "Operations";
        let mut ctx = Self {
            log_key: log_key.to_string(),
            stopped: Signal::new(
                bus.clone(),
                Base::new("stopped", Some("System state stopped")),
            ),
            starting: Signal::new(
                bus.clone(),
                Base::new("starting", Some("System state starting")),
            ),
            running: Signal::new(
                bus.clone(),
                Base::new("running", Some("System state running")),
            ),
            stopping: Signal::new(
                bus.clone(),
                Base::new("stopping", Some("System state stopping")),
            ),
            cleaning: Signal::new(
                bus.clone(),
                Base::new("cleaning", Some("System state cleaning")),
            ),
            emergency_out: Signal::new(
                bus.clone(),
                Base::new("emergency", Some("System state emergency")),
            ),
            fault_out: Signal::new(bus.clone(), Base::new("fault", Some("System state fault"))),
            maintenance_out: Signal::new(
                bus.clone(),
                Base::new("maintenance", Some("System state maintenance")),
            ),
            mode: Signal::new(bus.clone(), Base::new("mode", Some("Current system mode"))),
            mode_str: Signal::new(
                bus.clone(),
                Base::new("mode_str", Some("Current system mode as string")),
            ),
            stop_reason: Signal::new(
                bus.clone(),
                Base::new(
                    "stop_reason",
                    Some("Reason for system going into stopped state"),
                ),
            ),
            starting_finished: Slot::new(
                bus.clone(),
                Base::new(
                    "starting_finished",
                    Some("Starting finished, when certain initalization steps are done"),
                ),
            ),
            stopping_finished: Slot::new(
                bus.clone(),
                Base::new(
                    "stopping_finished",
                    Some("Stopping finished, when certain shutdown sequence is done"),
                ),
            ),
            run_button: Slot::new(
                bus.clone(),
                Base::new("run_button", Some("Run button pressed")),
            ),
            cleaning_button: Slot::new(
                bus.clone(),
                Base::new("cleaning_button", Some("Cleaning button pressed")),
            ),
            maintenance_button: Slot::new(
                bus.clone(),
                Base::new("maintenance_button", Some("Maintenance button pressed")),
            ),
            emergency_in: Slot::new(
                bus.clone(),
                Base::new(
                    "emergency",
                    Some("Emergency input signal, true when emergency is active"),
                ),
            ),
            fault_in: Slot::new(
                bus.clone(),
                Base::new(
                    "fault",
                    Some("Fault input signal, true when fault is active"),
                ),
            ),
            confman: confman::ConfMan::new(bus.clone(), "Operations"),
            sm_event_sender,
            mode_update_sender,
            starting_handle: None,
            stopping_handle: None,
            bus,
        };

        let sm_event_sender = ctx.sm_event_sender.clone();
        let log_key = ctx.log_key.clone();
        ctx.starting_finished.recv(Box::new(move |val: &bool| {
            trace!(target: &log_key, "Starting finished: {}", val);
            if *val {
                let _ = sm_event_sender
                    .send(OperationsEvents::StartingFinished)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send StartingFinished event: {}", e);
                    });
            }
        }));
        let sm_event_sender = ctx.sm_event_sender.clone();
        let log_key = ctx.log_key.clone();
        ctx.stopping_finished.recv(Box::new(move |val: &bool| {
            trace!(target: &log_key, "Stopping finished: {}", val);
            if *val {
                let _ = sm_event_sender
                    .send(OperationsEvents::StoppingFinished)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send StoppingFinished event: {}", e);
                    });
            }
        }));
        let sm_event_sender = ctx.sm_event_sender.clone();
        let log_key = ctx.log_key.clone();
        ctx.run_button.recv(Box::new(move |val: &bool| {
            trace!(target: &log_key, "Run button pressed: {}", val);
            if *val {
                let _ = sm_event_sender
                    .send(OperationsEvents::RunButton)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send RunButton event: {}", e);
                    });
            }
        }));
        let sm_event_sender = ctx.sm_event_sender.clone();
        let log_key = ctx.log_key.clone();
        ctx.cleaning_button.recv(Box::new(move |val: &bool| {
            trace!(target: &log_key, "Cleaning button pressed: {}", val);
            if *val {
                let _ = sm_event_sender
                    .send(OperationsEvents::CleaningButton)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send CleaningButton event: {}", e);
                    });
            }
        }));
        let sm_event_sender = ctx.sm_event_sender.clone();
        let log_key = ctx.log_key.clone();
        ctx.maintenance_button.recv(Box::new(move |val: &bool| {
            trace!(target: &log_key, "Maintenance button pressed: {}", val);
            if *val {
                let _ = sm_event_sender
                    .send(OperationsEvents::MaintenanceButton)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send MaintenanceButton event: {}", e);
                    });
            }
        }));
        let sm_event_sender = ctx.sm_event_sender.clone();
        let log_key = ctx.log_key.clone();
        ctx.emergency_in.recv(Box::new(move |val: &bool| {
            trace!(target: &log_key, "Emergency input signal: {}", val);
            if *val {
                let _ = sm_event_sender
                    .send(OperationsEvents::EmergencyOn)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send EmergencyOn event: {}", e);
                    });
            } else {
                let _ = sm_event_sender
                    .send(OperationsEvents::EmergencyOff)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send EmergencyOff event: {}", e);
                    });
            }
        }));
        let sm_event_sender = ctx.sm_event_sender.clone();
        let log_key = ctx.log_key.clone();
        ctx.fault_in.recv(Box::new(move |val: &bool| {
            trace!(target: &log_key, "Fault input signal: {}", val);
            if *val {
                let _ = sm_event_sender
                    .send(OperationsEvents::FaultOn)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send FaultOn event: {}", e);
                    });
            } else {
                let _ = sm_event_sender
                    .send(OperationsEvents::FaultOff)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send FaultOff event: {}", e);
                    });
            }
        }));

        #[cfg(feature = "dbus-expose")]
        ctx.dbus_expose();

        let _ = ctx.starting.send(false);
        let _ = ctx.stopping.send(false);
        let _ = ctx.cleaning.send(false);
        let _ = ctx.emergency_out.send(false);
        let _ = ctx.fault_out.send(false);
        let _ = ctx.maintenance_out.send(false);
        let _ = ctx.running.send(false);
        let _ = ctx.stopped.send(false);
        let _ = ctx.mode.send(OperationMode::Unknown as u64);
        let _ = ctx.mode_str.send("unknown".to_string());
        let _ = ctx.stop_reason.send("".to_string());

        let _ = ctx
            .sm_event_sender
            .send(OperationsEvents::SetStopped("init".to_string()));

        ctx
    }

    #[cfg(feature = "dbus-expose")]
    fn dbus_expose(&mut self) {
        tfc::ipc::dbus::SignalInterface::register(
            self.stopped.base(),
            self.bus.clone(),
            self.stopped.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.starting.base(),
            self.bus.clone(),
            self.starting.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.running.base(),
            self.bus.clone(),
            self.running.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.stopping.base(),
            self.bus.clone(),
            self.stopping.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.cleaning.base(),
            self.bus.clone(),
            self.cleaning.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.emergency_out.base(),
            self.bus.clone(),
            self.emergency_out.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.fault_out.base(),
            self.bus.clone(),
            self.fault_out.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.maintenance_out.base(),
            self.bus.clone(),
            self.maintenance_out.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.mode.base(),
            self.bus.clone(),
            self.mode.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.mode_str.base(),
            self.bus.clone(),
            self.mode_str.subscribe(),
        );
        tfc::ipc::dbus::SignalInterface::register(
            self.stop_reason.base(),
            self.bus.clone(),
            self.stop_reason.subscribe(),
        );

        tfc::ipc::dbus::SlotInterface::register(
            self.starting_finished.base(),
            self.bus.clone(),
            self.starting_finished.channel("dbus"),
        );
        tfc::ipc::dbus::SlotInterface::register(
            self.stopping_finished.base(),
            self.bus.clone(),
            self.stopping_finished.channel("dbus"),
        );
        tfc::ipc::dbus::SlotInterface::register(
            self.run_button.base(),
            self.bus.clone(),
            self.run_button.channel("dbus"),
        );
        tfc::ipc::dbus::SlotInterface::register(
            self.cleaning_button.base(),
            self.bus.clone(),
            self.cleaning_button.channel("dbus"),
        );
        tfc::ipc::dbus::SlotInterface::register(
            self.maintenance_button.base(),
            self.bus.clone(),
            self.maintenance_button.channel("dbus"),
        );
        tfc::ipc::dbus::SlotInterface::register(
            self.emergency_in.base(),
            self.bus.clone(),
            self.emergency_in.channel("dbus"),
        );
        tfc::ipc::dbus::SlotInterface::register(
            self.fault_in.base(),
            self.bus.clone(),
            self.fault_in.channel("dbus"),
        );
    }

    #[cfg(feature = "opcua-expose")]
    pub fn opcua_expose(
        &self,
        manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
        subscriptions: Arc<SubscriptionCache>,
        namespace: u16,
    ) {
        tfc::ipc::opcua::SignalInterface::new(
            self.stopped.base(),
            self.stopped.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.starting.base(),
            self.starting.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.running.base(),
            self.running.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.stopping.base(),
            self.stopping.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.cleaning.base(),
            self.cleaning.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.emergency_out.base(),
            self.emergency_out.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.fault_out.base(),
            self.fault_out.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.maintenance_out.base(),
            self.maintenance_out.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.mode.base(),
            self.mode.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.mode_str.base(),
            self.mode_str.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SignalInterface::new(
            self.stop_reason.base(),
            self.stop_reason.subscribe(),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();

        tfc::ipc::opcua::SlotInterface::new(
            self.starting_finished.base(),
            self.starting_finished.channel("opcua"),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SlotInterface::new(
            self.stopping_finished.base(),
            self.stopping_finished.channel("opcua"),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SlotInterface::new(
            self.run_button.base(),
            self.run_button.channel("opcua"),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SlotInterface::new(
            self.cleaning_button.base(),
            self.cleaning_button.channel("opcua"),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SlotInterface::new(
            self.maintenance_button.base(),
            self.maintenance_button.channel("opcua"),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SlotInterface::new(
            self.emergency_in.base(),
            self.emergency_in.channel("opcua"),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
        tfc::ipc::opcua::SlotInterface::new(
            self.fault_in.base(),
            self.fault_in.channel("opcua"),
            manager.clone(),
            subscriptions.clone(),
            namespace,
        )
        .register();
    }

    fn transition(
        &self,
        from: &OperationsStates,
        to: &OperationsStates,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        trace!(target: &self.log_key, "Transitioning from {:?} to {:?}", from, to);

        let new_mode = OperationMode::from(to);
        let old_mode = OperationMode::from(from);
        self.mode.send(new_mode as u64)?;
        self.mode_str.send(new_mode.to_string())?;
        self.mode_update_sender
            .send(OperationsUpdate { old_mode, new_mode })?;

        Ok(())
    }
    fn update_stop_reason(&mut self, reason: &str) {
        let _ = self.stop_reason.send(reason.to_string());
    }
}

impl OperationsStateMachineContext for OperationsImpl {
    fn on_entry_stopped(&mut self) {
        let _ = self.stopped.send(true);
    }
    fn on_exit_stopped(&mut self) {
        let _ = self.stopped.send(false);
        let _ = self.stop_reason.send("".to_string());
    }
    fn on_entry_starting(&mut self) {
        let _ = self.starting.send(true);

        let log_key = self.log_key.clone();
        match self.confman.read().startup_time {
            Some(time) => {
                let sm_event_sender = self.sm_event_sender.clone();
                self.starting_handle.replace(tokio::spawn(async move {
                    tokio::time::sleep(time.into()).await;
                    let _ = sm_event_sender
                        .send(OperationsEvents::StartingTimeout)
                        .map_err(|e| {
                            warn!(target: &log_key, "Failed to send StartingTimeout event: {}", e);
                        });
                }));
            }
            None => {
                let _ = self
                    .sm_event_sender
                    .send(OperationsEvents::StartingTimeout)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send StartingTimeout event: {}", e);
                    });
            }
        }
    }
    fn on_exit_starting(&mut self) {
        if let Some(handle) = self.starting_handle.take() {
            handle.abort();
        }
        let _ = self.starting.send(false);
    }
    fn on_entry_running(&mut self) {
        let _ = self.running.send(true);
    }
    fn on_exit_running(&mut self) {
        let _ = self.running.send(false);
    }
    fn on_entry_stopping(&mut self) {
        let _ = self.stopping.send(true);

        let log_key = self.log_key.clone();
        match self.confman.read().stopping_time {
            Some(time) => {
                let sm_event_sender = self.sm_event_sender.clone();
                self.stopping_handle.replace(tokio::spawn(async move {
                    tokio::time::sleep(time.into()).await;
                    let _ = sm_event_sender
                        .send(OperationsEvents::StoppingTimeout)
                        .map_err(|e| {
                            warn!(target: &log_key, "Failed to send StoppingTimeout event: {}", e);
                        });
                }));
            }
            None => {
                let _ = self
                    .sm_event_sender
                    .send(OperationsEvents::StoppingTimeout)
                    .map_err(|e| {
                        warn!(target: &log_key, "Failed to send StoppingTimeout event: {}", e);
                    });
            }
        }
    }
    fn on_exit_stopping(&mut self) {
        if let Some(handle) = self.stopping_handle.take() {
            handle.abort();
        }
        let _ = self.stopping.send(false);
    }
    fn on_entry_cleaning(&mut self) {
        let _ = self.cleaning.send(true);
    }
    fn on_exit_cleaning(&mut self) {
        let _ = self.cleaning.send(false);
    }
    fn on_entry_emergency(&mut self) {
        let _ = self.emergency_out.send(true);
    }
    fn on_exit_emergency(&mut self) {
        let _ = self.emergency_out.send(false);
    }
    fn on_entry_fault(&mut self) {
        let _ = self.fault_out.send(true);
    }
    fn on_exit_fault(&mut self) {
        let _ = self.fault_out.send(false);
    }
    fn on_entry_maintenance(&mut self) {
        let _ = self.maintenance_out.send(true);
    }
    fn on_exit_maintenance(&mut self) {
        let _ = self.maintenance_out.send(false);
    }
    fn is_fault(&self) -> Result<bool, ()> {
        Ok(self.fault_in.subscribe().borrow().clone().unwrap_or(false))
    }
    fn transition_callback(&self, exit: &OperationsStates, entry: &OperationsStates) {
        let _ = self.transition(exit, entry);
    }
    fn log_process_event(&self, last_state: &OperationsStates, event: &OperationsEvents) {
        trace!(target: &self.log_key,
            "[StateMachineLogger]\t[{:?}] Processing event {:?}",
            last_state, event
        );
    }
}
