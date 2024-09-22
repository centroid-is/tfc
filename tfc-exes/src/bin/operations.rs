use derive_more::Display;
use serde::{Deserialize, Serialize};
use smlang::statemachine;
use std::fmt::Debug;
use tfc::ipc::{Base, Signal, Slot};
use tfc::progbase;
use tokio;
use zbus::interface;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, zbus::zvariant::Type)]
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

struct OperationsClient {
    log_key: String,
}

impl OperationsClient {
    pub fn new(key: &str) -> Self {
        Self {
            log_key: key.to_string(),
        }
    }
}
#[interface(name = "is.centroid.OperationMode")]
impl OperationsClient {
    #[zbus(property)]
    async fn mode(&self) -> Result<String, zbus::fdo::Error> {
        Ok("manual".to_string())
    }

    #[zbus(signal)]
    async fn update(
        signal_ctxt: &zbus::SignalContext<'_>,
        new_mode: &str,
        old_mode: &str,
    ) -> zbus::Result<()> {
        println!(
            "Operation mode updated from {:?} to {:?}",
            old_mode, new_mode
        );
        Ok(())
    }

    fn set_mode(&self, mode: &str) -> Result<(), zbus::fdo::Error> {
        println!("Setting operation mode to {:?}", mode);
        Ok(())
    }

    fn stop_with_reason(&self, reason: &str) -> Result<(), zbus::fdo::Error> {
        println!("Stopping with reason: {}", reason);
        Ok(())
    }
}

// Define the state machine transitions and actions
statemachine! {
    name: Operations,
    derive_states: [Debug, Display],
    derive_events: [Debug, Display],
    transitions: {
        *Init + SetStopped / transition_to_stopped = Stopped,

        Stopped + SetStarting / transition_to_starting = Starting,
        Stopped + RunButton / transition_to_starting = Starting,
        Starting + StartingTimeout / transition_to_running = Running,
        Starting + StartingFinished / transition_to_running = Running,
        Running + RunButton / transition_to_stopping = Stopping,
        Running + SetStopped / transition_to_stopping = Stopping,
        Stopping + StoppingTimeout / transition_to_stopped = Stopped,
        Stopping + StoppingFinished / transition_to_stopped = Stopped,

        Stopped + CleaningButton / transition_to_cleaning = Cleaning,
        Stopped + SetCleaning / transition_to_cleaning = Cleaning,
        Cleaning + CleaningButton / transition_to_stopped = Stopped,
        Cleaning + SetStopped / transition_to_stopped = Stopped,

        // Transitions to Emergency
        Stopped + SetEmergency / transition_to_emergency = Emergency,
        Stopping + SetEmergency / transition_to_emergency = Emergency,
        Starting + SetEmergency / transition_to_emergency = Emergency,
        Running + SetEmergency / transition_to_emergency = Emergency,
        Cleaning + SetEmergency / transition_to_emergency = Emergency,
        Fault + SetEmergency / transition_to_emergency = Emergency,
        Maintenance + SetEmergency / transition_to_emergency = Emergency,

        Stopped + EmergencyOn / transition_to_emergency = Emergency,
        Stopping + EmergencyOn / transition_to_emergency = Emergency,
        Starting + EmergencyOn / transition_to_emergency = Emergency,
        Running + EmergencyOn / transition_to_emergency = Emergency,
        Cleaning + EmergencyOn / transition_to_emergency = Emergency,
        Fault + EmergencyOn / transition_to_emergency = Emergency,
        Maintenance + EmergencyOn / transition_to_emergency = Emergency,

        Emergency + EmergencyOff [!is_fault] / transition_to_stopped = Stopped,
        Emergency + EmergencyOff [is_fault] / transition_to_fault = Fault,

        Stopped + FaultOn / transition_to_fault = Fault,
        Stopped + SetFault / transition_to_fault = Fault,
        Running + FaultOn / transition_to_fault = Fault,
        Running + SetFault / transition_to_fault = Fault,

        Fault + FaultOff / transition_to_stopped = Stopped,

        Stopped + MaintenanceButton / transition_to_maintenance = Maintenance,
        Stopped + SetMaintenance / transition_to_maintenance = Maintenance,
        Maintenance + MaintenanceButton / transition_to_stopped = Stopped,
        Maintenance + SetStopped / transition_to_stopped = Stopped,
    },
}

// Define the owner struct with required methods
pub struct Context {
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
    starting_finished: Slot<bool>,
    stopping_finished: Slot<bool>,
    run_button: Slot<bool>,
    cleaning_button: Slot<bool>,
    maintenance_button: Slot<bool>,
    emergency_in: Slot<bool>,
    fault_in: Slot<bool>,
}

impl Context {
    pub fn new(bus: zbus::Connection) -> Self {
        let bus_cp = bus.clone();
        let path = "/is/centroid/OperationMode";
        let client = OperationsClient::new("OperationMode");
        let log_key = "OperationMode";
        tokio::spawn(async move {
            // log if error
            let _ = bus_cp
                .object_server()
                .at(path, client)
                .await
                .expect(&format!("Error registering object: {}", log_key));
        });
        Self {
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
                    "emergency_in",
                    Some("Emergency input signal, true when emergency is active"),
                ),
            ),
            fault_in: Slot::new(
                bus.clone(),
                Base::new(
                    "fault_in",
                    Some("Fault input signal, true when fault is active"),
                ),
            ),
        }
    }
}
impl OperationsStateMachineContext for Context {
    fn on_entry_stopped(&mut self) {
        println!("Entering Stopped state");
    }
    fn on_exit_stopped(&mut self) {
        println!("Exiting Stopped state");
    }
    fn on_entry_starting(&mut self) {
        println!("Entering Starting state");
    }
    fn on_exit_starting(&mut self) {
        println!("Exiting Starting state");
    }
    fn on_entry_running(&mut self) {
        println!("Entering Running state");
    }
    fn on_exit_running(&mut self) {
        println!("Exiting Running state");
    }
    fn on_entry_stopping(&mut self) {
        println!("Entering Stopping state");
    }
    fn on_exit_stopping(&mut self) {
        println!("Exiting Stopping state");
    }
    fn on_entry_cleaning(&mut self) {
        println!("Entering Cleaning state");
    }
    fn on_exit_cleaning(&mut self) {
        println!("Exiting Cleaning state");
    }
    fn on_entry_emergency(&mut self) {
        println!("Entering Emergency state");
    }
    fn on_exit_emergency(&mut self) {
        println!("Exiting Emergency state");
    }
    fn on_entry_fault(&mut self) {
        println!("Entering Fault state");
    }
    fn on_exit_fault(&mut self) {
        println!("Exiting Fault state");
    }
    fn on_entry_maintenance(&mut self) {
        println!("Entering Maintenance state");
    }
    fn on_exit_maintenance(&mut self) {
        println!("Exiting Maintenance state");
    }

    fn is_fault(&self) -> Result<bool, ()> {
        // Implement your fault condition check here
        Ok(false)
    }
    // Transition actions
    fn transition_to_stopped(&mut self) -> Result<(), ()> {
        let to_state = OperationsStates::Stopped;
        Ok(())
    }
    fn transition_to_starting(&mut self) -> Result<(), ()> {
        let to_state = OperationsStates::Starting;
        Ok(())
    }
    fn transition_to_running(&mut self) -> Result<(), ()> {
        let to_state = OperationsStates::Running;
        Ok(())
    }
    fn transition_to_stopping(&mut self) -> Result<(), ()> {
        let to_state = OperationsStates::Stopping;
        Ok(())
    }
    fn transition_to_cleaning(&mut self) -> Result<(), ()> {
        let to_state = OperationsStates::Cleaning;
        Ok(())
    }

    fn transition_to_emergency(&mut self) -> Result<(), ()> {
        let to_state = OperationsStates::Emergency;
        Ok(())
    }
    fn transition_to_fault(&mut self) -> Result<(), ()> {
        let to_state = OperationsStates::Fault;
        Ok(())
    }
    fn transition_to_maintenance(&mut self) -> Result<(), ()> {
        let to_state = OperationsStates::Maintenance;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    progbase::init();
    let formatted_name = format!(
        "is.centroid.{}.{}",
        progbase::exe_name(),
        progbase::proc_name()
    );
    let bus = zbus::connection::Builder::system()?
        .name(formatted_name)?
        .build()
        .await?;
    let ctx: Context = Context::new(bus.clone());
    let mut sm = OperationsStateMachine::new(ctx);

    println!("Starting state machine");
    println!("Current state: {:?}", sm.state());
    // Start the state machine by processing the initial event
    sm.process_event(OperationsEvents::SetStopped).unwrap();
    println!("Current state: {:?}", sm.state());
    // Process events as needed
    sm.process_event(OperationsEvents::RunButton).unwrap();
    sm.process_event(OperationsEvents::StartingFinished)
        .unwrap();
    sm.process_event(OperationsEvents::SetEmergency).unwrap();
    sm.process_event(OperationsEvents::EmergencyOff).unwrap();
    sm.process_event(OperationsEvents::FaultOn).unwrap();
    sm.process_event(OperationsEvents::FaultOff).unwrap();
    sm.process_event(OperationsEvents::MaintenanceButton)
        .unwrap();
    sm.process_event(OperationsEvents::SetStopped).unwrap();
    std::future::pending::<()>().await;
    Ok(())
}
