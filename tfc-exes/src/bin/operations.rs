use derive_more::Display;
use futures::SinkExt;
use futures::StreamExt;
use futures_channel::mpsc;
use log::{info, trace};
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use smlang::statemachine;
use std::error::Error;
use std::fmt::Debug;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tfc::confman;
use tfc::ipc::{Base, Signal, Slot};
use tfc::logger;
use tfc::progbase;
use tokio;
use tokio::task::JoinHandle;
use zbus::interface;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, zbus::zvariant::Type, Display,
)]
pub enum OperationMode {
    Unknown = 0,
    Stopped = 1,
    Starting = 2,
    Running = 3,
    Stopping = 4,
    Cleaning = 5,
    Emergency = 6,
    Fault = 7,
    Maintenance = 8,
}

impl FromStr for OperationMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s_lower = s.to_lowercase();
        match s_lower.as_str() {
            "unknown" => Ok(OperationMode::Unknown),
            "stopped" => Ok(OperationMode::Stopped),
            "starting" => Ok(OperationMode::Starting),
            "running" => Ok(OperationMode::Running),
            "stopping" => Ok(OperationMode::Stopping),
            "cleaning" => Ok(OperationMode::Cleaning),
            "emergency" => Ok(OperationMode::Emergency),
            "fault" => Ok(OperationMode::Fault),
            "maintenance" => Ok(OperationMode::Maintenance),
            _ => Err(()),
        }
    }
}

struct OperationsClient {
    log_key: String,
    new_mode_channel: mpsc::Sender<OperationMode>,
    stop_channel: mpsc::Sender<String>,
    current_mode: Arc<Mutex<OperationMode>>,
}

impl OperationsClient {
    pub fn new(
        current_mode: Arc<Mutex<OperationMode>>,
        new_mode_channel: mpsc::Sender<OperationMode>,
        stop_channel: mpsc::Sender<String>,
        key: &str,
    ) -> Self {
        Self {
            log_key: key.to_string(),
            new_mode_channel,
            stop_channel,
            current_mode,
        }
    }
}
#[interface(name = "is.centroid.OperationMode")]
impl OperationsClient {
    #[zbus(property)]
    async fn mode(&self) -> Result<String, zbus::fdo::Error> {
        Ok(self.current_mode.lock().to_string())
    }

    #[zbus(signal)]
    async fn update(
        signal_ctxt: &zbus::SignalContext<'_>,
        new_mode: &str,
        old_mode: &str,
    ) -> zbus::Result<()>;

    async fn set_mode(&mut self, mode_str: &str) -> Result<(), zbus::fdo::Error> {
        info!(target: &self.log_key, "Setting operation mode to {:?}", mode_str);
        let mode: OperationMode = mode_str.parse().map_or(OperationMode::Unknown, |mode| mode);
        self.new_mode_channel.send(mode).await.map_err(|e| {
            let err_msg = format!("Error sending new mode: {}", e);
            info!(target: &self.log_key, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })?;
        Ok(())
    }

    async fn stop_with_reason(&mut self, reason: &str) -> Result<(), zbus::fdo::Error> {
        self.stop_channel
            .send(reason.to_string())
            .await
            .map_err(|e| {
                let err_msg = format!("Error sending stop: {}", e);
                info!(target: &self.log_key, "{}", err_msg);
                zbus::fdo::Error::Failed(err_msg)
            })?;
        Ok(())
    }
}

// Define the state machine transitions and actions
statemachine! {
    name: Operations,
    derive_states: [Debug, Display, Clone],
    derive_events: [Debug, Display],
    transitions: {
        *Init + SetStopped = Stopped,

        Stopped + SetStarting = Starting,
        Stopped + RunButton = Starting,
        Starting + StartingTimeout = Running,
        Starting + StartingFinished = Running,
        Running + RunButton = Stopping,
        Running + SetStopped = Stopping,
        Stopping + StoppingTimeout = Stopped,
        Stopping + StoppingFinished = Stopped,

        Stopped + CleaningButton = Cleaning,
        Stopped + SetCleaning = Cleaning,
        Cleaning + CleaningButton = Stopped,
        Cleaning + SetStopped = Stopped,

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
        Maintenance + SetStopped = Stopped,
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

#[derive(Default, Serialize, Deserialize, JsonSchema)]
struct Storage {
    #[schemars(description = "Delay to run initial sequences to get the equipment ready.")]
    startup_time: Option<Duration>,
    #[schemars(
        description = "Delay to run shutdown sequences to get the equipment ready for being stopped."
    )]
    stopping_time: Option<Duration>,
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
    sm: Arc<Mutex<Option<OperationsStateMachine<Context>>>>,
    confman: confman::ConfMan<Storage>,
    dbus_path: String,
    bus: zbus::Connection,
    current_mode: Arc<Mutex<OperationMode>>,
    stop_channel: Arc<Mutex<mpsc::Receiver<String>>>,
    stop_task: Option<JoinHandle<()>>,
    new_mode_channel: Arc<Mutex<mpsc::Receiver<OperationMode>>>,
    new_mode_task: Option<JoinHandle<()>>,
}

impl OperationsImpl {
    pub fn new(bus: zbus::Connection) -> Arc<Mutex<Self>> {
        let bus_cp = bus.clone();
        let path = "/is/centroid/OperationMode";
        let current_mode = Arc::new(Mutex::new(OperationMode::Unknown));
        let (new_mode_tx, new_mode_rx) = mpsc::channel(10);
        let (stop_tx, stop_rx) = mpsc::channel(10);
        let client = OperationsClient::new(
            Arc::clone(&current_mode),
            new_mode_tx,
            stop_tx,
            "OperationMode",
        );
        let log_key = "OperationMode";
        tokio::spawn(async move {
            // log if error
            let _ = bus_cp
                .object_server()
                .at(path, client)
                .await
                .expect(&format!("Error registering object: {}", log_key));
        });
        let ctx = Arc::new(Mutex::new(Self {
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
            sm: Arc::new(Mutex::new(None)),
            confman: confman::ConfMan::new(bus.clone(), "Operations").with_default(Storage {
                startup_time: Some(Duration::from_secs(0)),
                stopping_time: Some(Duration::from_secs(0)),
            }),
            dbus_path: path.to_string(),
            bus,
            current_mode,
            stop_channel: Arc::new(Mutex::new(stop_rx)),
            stop_task: None,
            new_mode_channel: Arc::new(Mutex::new(new_mode_rx)),
            new_mode_task: None,
        }));

        let mut ops_impl = ctx.lock();
        let ops_impl_ptr: *mut OperationsImpl = &mut *ops_impl as *mut OperationsImpl;
        drop(ops_impl);
        let context = Context {
            log_key: "StateMachine".to_string(),
            owner: ops_impl_ptr,
        };

        ctx.lock()
            .sm
            .lock()
            .replace(OperationsStateMachine::new(context));

        let shared_stop_channel = Arc::clone(&ctx.lock().stop_channel);
        let shared_ctx = Arc::clone(&ctx);
        ctx.lock().stop_task.replace(tokio::spawn(async move {
            let mut stop_channel = shared_stop_channel.lock();
            while let Some(reason) = stop_channel.next().await {
                shared_ctx.lock().on_stop_reason(&reason);
            }
        }));

        let shared_new_mode_channel = Arc::clone(&ctx.lock().new_mode_channel);
        let shared_ctx = Arc::clone(&ctx);
        ctx.lock().new_mode_task.replace(tokio::spawn(async move {
            let mut new_mode_channel = shared_new_mode_channel.lock();
            while let Some(mode) = new_mode_channel.next().await {
                shared_ctx.lock().on_new_mode(mode);
            }
        }));

        let shared_ctx = Arc::clone(&ctx);
        ctx.lock()
            .starting_finished
            .recv(Box::new(move |val: &bool| {
                shared_ctx.lock().on_starting_finished(val);
            }));
        let shared_ctx = Arc::clone(&ctx);
        ctx.lock()
            .stopping_finished
            .recv(Box::new(move |val: &bool| {
                shared_ctx.lock().on_stopping_finished(val);
            }));
        let shared_ctx = Arc::clone(&ctx);
        ctx.lock().run_button.recv(Box::new(move |val: &bool| {
            shared_ctx.lock().on_run_button(val);
        }));
        let shared_ctx = Arc::clone(&ctx);
        ctx.lock().cleaning_button.recv(Box::new(move |val: &bool| {
            shared_ctx.lock().on_cleaning_button(val);
        }));
        let shared_ctx = Arc::clone(&ctx);
        ctx.lock()
            .maintenance_button
            .recv(Box::new(move |val: &bool| {
                shared_ctx.lock().on_maintenance_button(val);
            }));
        let shared_ctx = Arc::clone(&ctx);
        ctx.lock().emergency_in.recv(Box::new(move |val: &bool| {
            shared_ctx.lock().on_emergency_in(val);
        }));
        let shared_ctx = Arc::clone(&ctx);
        ctx.lock().fault_in.recv(Box::new(move |val: &bool| {
            shared_ctx.lock().on_fault_in(val);
        }));

        let _ = ctx.lock().starting.send(false);
        let _ = ctx.lock().stopping.send(false);
        let _ = ctx.lock().cleaning.send(false);
        let _ = ctx.lock().emergency_out.send(false);
        let _ = ctx.lock().fault_out.send(false);
        let _ = ctx.lock().maintenance_out.send(false);
        let _ = ctx.lock().running.send(false);
        let _ = ctx.lock().stopped.send(false);
        let _ = ctx.lock().mode.send(OperationMode::Unknown as u64);
        let _ = ctx.lock().mode_str.send("unknown".to_string());
        let _ = ctx.lock().stop_reason.send("".to_string());

        {
            // todo should we just stop the init state?
            // it's nice to call transition and so forth
            let _ = ctx
                .lock()
                .sm
                .lock()
                .as_mut()
                .unwrap()
                .process_event(OperationsEvents::SetStopped)
                .map_err(|e| info!(target: log_key,"Error processing event: {:?}", e));
        }

        ctx
    }

    fn on_starting_finished(&mut self, val: &bool) {
        trace!(target: &self.log_key, "Starting finished: {}", val);
        if !val {
            return;
        }

        let _ = self
            .sm
            .lock()
            .as_mut()
            .unwrap()
            .process_event(OperationsEvents::StartingFinished)
            .map_err(|e| info!(target: &self.log_key,"Error processing event: {:?}", e));
    }
    fn on_stopping_finished(&mut self, val: &bool) {
        trace!(target: &self.log_key, "Stopping finished: {}", val);
        if !val {
            return;
        }

        let _ = self
            .sm
            .lock()
            .as_mut()
            .unwrap()
            .process_event(OperationsEvents::StoppingFinished)
            .map_err(|e| info!(target: &self.log_key,"Error processing event: {:?}", e));
    }
    fn on_run_button(&mut self, val: &bool) {
        trace!(target: &self.log_key, "Run button pressed: {}", val);
        if !val {
            return;
        }

        let _ = self
            .sm
            .lock()
            .as_mut()
            .unwrap()
            .process_event(OperationsEvents::RunButton)
            .map_err(|e| info!(target: &self.log_key,"Error processing event: {:?}", e));
    }
    fn on_cleaning_button(&mut self, val: &bool) {
        trace!(target: &self.log_key, "Cleaning button pressed: {}", val);
        if !val {
            return;
        }

        let _ = self
            .sm
            .lock()
            .as_mut()
            .unwrap()
            .process_event(OperationsEvents::CleaningButton)
            .map_err(|e| info!(target: &self.log_key,"Error processing event: {:?}", e));
    }
    fn on_maintenance_button(&mut self, val: &bool) {
        trace!(target: &self.log_key, "Maintenance button pressed: {}", val);
        if !val {
            return;
        }

        let _ = self
            .sm
            .lock()
            .as_mut()
            .unwrap()
            .process_event(OperationsEvents::MaintenanceButton)
            .map_err(|e| info!(target: &self.log_key,"Error processing event: {:?}", e));
    }
    fn on_emergency_in(&mut self, val: &bool) {
        trace!(target: &self.log_key, "Emergency input signal: {}", val);
        if *val {
            let _ = self
                .sm
                .lock()
                .as_mut()
                .unwrap()
                .process_event(OperationsEvents::EmergencyOn)
                .map_err(|e| info!(target: &self.log_key,"Error processing event: {:?}", e));
        } else {
            let _ = self
                .sm
                .lock()
                .as_mut()
                .unwrap()
                .process_event(OperationsEvents::EmergencyOff)
                .map_err(|e| info!(target: &self.log_key,"Error processing event: {:?}", e));
        }
    }
    fn on_fault_in(&mut self, val: &bool) {
        trace!(target: &self.log_key, "Fault input signal: {}", val);
        if *val {
            let _ = self
                .sm
                .lock()
                .as_mut()
                .unwrap()
                .process_event(OperationsEvents::FaultOn)
                .map_err(|e| info!(target: &self.log_key,"Error processing event: {:?}", e));
        } else {
            let _ = self
                .sm
                .lock()
                .as_mut()
                .unwrap()
                .process_event(OperationsEvents::FaultOff)
                .map_err(|e| info!(target: &self.log_key,"Error processing event: {:?}", e));
        }
    }
    fn on_stop_reason(&mut self, reason: &str) {
        trace!(target: &self.log_key, "Stop reason: {}", reason);
        let _ = self.stop_reason.send(reason.to_string());
        let _ = self
            .sm
            .lock()
            .as_mut()
            .unwrap()
            .process_event(OperationsEvents::SetStopped);
    }
    fn on_new_mode(&mut self, mode: OperationMode) {
        trace!(target: &self.log_key, "New mode: {}", mode);
        match mode {
            OperationMode::Unknown => {}
            OperationMode::Stopped => {
                let _ = self
                    .sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::SetStopped);
            }
            OperationMode::Starting => {
                let _ = self
                    .sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::SetStarting);
            }
            OperationMode::Running => {
                let _ = self
                    .sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::SetStarting);
            }
            OperationMode::Stopping => {
                let _ = self
                    .sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::SetStopped);
            }
            OperationMode::Cleaning => {
                let _ = self
                    .sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::SetCleaning);
            }
            OperationMode::Emergency => {
                let _ = self
                    .sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::SetEmergency);
            }
            OperationMode::Fault => {
                let _ = self
                    .sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::SetFault);
            }
            OperationMode::Maintenance => {
                let _ = self
                    .sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::SetMaintenance);
            }
        }
    }

    // State machine callbacks
    fn on_entry_stopped(&mut self) {
        let _ = self.stopped.send(true);
    }
    fn on_exit_stopped(&mut self) {
        let _ = self.stopped.send(false);
        let _ = self.stop_reason.send("".to_string());
    }
    fn on_entry_starting(&mut self) {
        let _ = self.starting.send(true);

        let sm = Arc::clone(&self.sm);
        if let Some(startup_time) = self.confman.read().startup_time {
            tokio::spawn(async move {
                tokio::time::sleep(startup_time).await;
                let _ = sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::StartingTimeout);
            });
        }
    }
    fn on_exit_starting(&mut self) {
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

        let sm = Arc::clone(&self.sm);
        if let Some(stopping_time) = self.confman.read().stopping_time {
            tokio::spawn(async move {
                tokio::time::sleep(stopping_time).await;
                let _ = sm
                    .lock()
                    .as_mut()
                    .unwrap()
                    .process_event(OperationsEvents::StoppingTimeout);
            });
        }
    }
    fn on_exit_stopping(&mut self) {
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

    fn is_fault(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        self.fault_in.value()
    }

    fn transition(&mut self, from: &OperationsStates, to: &OperationsStates) -> Result<(), ()> {
        trace!(target: &self.log_key, "Transitioning from {:?} to {:?}", from, to);

        let new_mode = OperationMode::from(to);
        let old_mode = OperationMode::from(from);
        let _ = self.mode.send(new_mode as u64);
        let _ = self.mode_str.send(new_mode.to_string());

        {
            let mut current_mode = self.current_mode.lock();
            *current_mode = new_mode;
        }

        let shared_bus = self.bus.clone();
        let shared_dbus_path = self.dbus_path.clone();
        let new_mode_str = new_mode.to_string();
        let old_mode_str = old_mode.to_string();
        tokio::spawn(async move {
            let iface: zbus::InterfaceRef<OperationsClient> = shared_bus
                .object_server()
                .interface(shared_dbus_path)
                .await
                .unwrap();
            let _ = OperationsClient::update(&iface.signal_context(), &new_mode_str, &old_mode_str)
                .await;
        });
        Ok(())
    }
}

impl Drop for OperationsImpl {
    fn drop(&mut self) {
        info!(target: &self.log_key, "Dropping OperationsImpl");
        let _ = self.stop_task.take().unwrap().abort();
        let _ = self.new_mode_task.take().unwrap().abort();
    }
}

pub struct Context {
    owner: *mut OperationsImpl,
    log_key: String,
}
// todo: make this safe
unsafe impl Send for Context {}
unsafe impl Sync for Context {}

impl Context {
    fn with_owner<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut OperationsImpl) -> R,
    {
        // this should be safe, this is just circular dependency hell
        // todo: make this safe
        unsafe {
            let ops_impl = &mut *self.owner;
            f(ops_impl)
        }
    }
}

impl OperationsStateMachineContext for Context {
    fn on_entry_stopped(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_entry_stopped();
        });
    }
    fn on_exit_stopped(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_exit_stopped();
        });
    }
    fn on_entry_starting(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_entry_starting();
        });
    }
    fn on_exit_starting(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_exit_starting();
        });
    }
    fn on_entry_running(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_entry_running();
        });
    }
    fn on_exit_running(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_exit_running();
        });
    }
    fn on_entry_stopping(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_entry_stopping();
        });
    }
    fn on_exit_stopping(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_exit_stopping();
        });
    }
    fn on_entry_cleaning(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_entry_cleaning();
        });
    }
    fn on_exit_cleaning(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_exit_cleaning();
        });
    }
    fn on_entry_emergency(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_entry_emergency();
        });
    }
    fn on_exit_emergency(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_exit_emergency();
        });
    }
    fn on_entry_fault(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_entry_fault();
        });
    }
    fn on_exit_fault(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_exit_fault();
        });
    }
    fn on_entry_maintenance(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_entry_maintenance();
        });
    }
    fn on_exit_maintenance(&mut self) {
        self.with_owner(|ops_impl| {
            ops_impl.on_exit_maintenance();
        });
    }
    fn is_fault(&self) -> Result<bool, ()> {
        // returns false if there is an error
        Ok(self
            .with_owner(|ops_impl| {
                ops_impl.is_fault().map_err(|e| {
                    info!(target: &self.log_key, "Error getting fault state: {}", e);
                    ()
                })
            })
            .unwrap_or(false))
    }
    fn transition_callback(&self, exit: &OperationsStates, entry: &OperationsStates) {
        trace!(target: &self.log_key, "Transition from {:?}. to: {:?}", exit, entry);
        self.with_owner(|ops_impl| {
            let _ = ops_impl.transition(exit, entry);
        });
    }
    fn log_process_event(&self, last_state: &OperationsStates, event: &OperationsEvents) {
        trace!(target: &self.log_key,
            "[StateMachineLogger]\t[{:?}] Processing event {:?}",
            last_state, event
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    progbase::init();
    logger::init_combined_logger()?;
    let formatted_name = format!(
        "is.centroid.{}.{}",
        progbase::exe_name(),
        progbase::proc_name()
    );
    let bus = zbus::connection::Builder::system()?
        .name(formatted_name)?
        .build()
        .await?;

    let _ = Arc::new(Mutex::new(OperationsImpl::new(bus.clone())));

    std::future::pending::<()>().await;
    Ok(())
}
