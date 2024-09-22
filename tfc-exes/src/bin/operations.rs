use derive_more::Display;
use smlang::statemachine;
use std::fmt::Debug;

// Define the state machine transitions and actions
statemachine! {
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
    // Add any necessary fields here
}

impl Context {
    fn on_enter_stopped(&mut self) {
        println!("Entering Stopped state");
    }
    fn on_exit_stopped(&mut self) {
        println!("Exiting Stopped state");
    }
    fn on_enter_starting(&mut self) {
        println!("Entering Starting state");
    }
    fn on_exit_starting(&mut self) {
        println!("Exiting Starting state");
    }
    fn on_enter_running(&mut self) {
        println!("Entering Running state");
    }
    fn on_exit_running(&mut self) {
        println!("Exiting Running state");
    }
    fn on_enter_stopping(&mut self) {
        println!("Entering Stopping state");
    }
    fn on_exit_stopping(&mut self) {
        println!("Exiting Stopping state");
    }
    fn on_enter_cleaning(&mut self) {
        println!("Entering Cleaning state");
    }
    fn on_exit_cleaning(&mut self) {
        println!("Exiting Cleaning state");
    }
    fn on_enter_emergency(&mut self) {
        println!("Entering Emergency state");
    }
    fn on_exit_emergency(&mut self) {
        println!("Exiting Emergency state");
    }
    fn on_enter_fault(&mut self) {
        println!("Entering Fault state");
    }
    fn on_exit_fault(&mut self) {
        println!("Exiting Fault state");
    }
    fn on_enter_maintenance(&mut self) {
        println!("Entering Maintenance state");
    }
    fn on_exit_maintenance(&mut self) {
        println!("Exiting Maintenance state");
    }

    fn transition(&mut self, to_state: States, from_state: States) {
        println!("Transition from {:?} to {:?}", from_state, to_state);
    }

    fn is_fault(&self) -> bool {
        // Implement your fault condition check here
        false
    }
}

impl StateMachineContext for Context {
    // Transition actions
    fn transition_to_stopped(&mut self) -> Result<(), ()> {
        let to_state = States::Stopped;
        Ok(())
    }
    fn transition_to_starting(&mut self) -> Result<(), ()> {
        let to_state = States::Starting;
        Ok(())
    }
    fn transition_to_running(&mut self) -> Result<(), ()> {
        let to_state = States::Running;
        Ok(())
    }
    fn transition_to_stopping(&mut self) -> Result<(), ()> {
        let to_state = States::Stopping;
        Ok(())
    }
    fn transition_to_cleaning(&mut self) -> Result<(), ()> {
        let to_state = States::Cleaning;
        Ok(())
    }

    fn transition_to_emergency(&mut self) -> Result<(), ()> {
        let to_state = States::Emergency;
        Ok(())
    }
    fn transition_to_fault(&mut self) -> Result<(), ()> {
        let to_state = States::Fault;
        Ok(())
    }
    fn transition_to_maintenance(&mut self) -> Result<(), ()> {
        let to_state = States::Maintenance;
        Ok(())
    }

    fn is_fault(&self) -> Result<bool, ()> {
        // Implement your fault condition check here
        Ok(false)
    }
}

fn main() {
    let ctx: Context = Context {};
    let mut sm = StateMachine::new(ctx);

    println!("Starting state machine");
    println!("Current state: {:?}", sm.state());
    // Start the state machine by processing the initial event
    sm.process_event(Events::SetStopped).unwrap();
    println!("Current state: {:?}", sm.state());
    // Process events as needed
    sm.process_event(Events::RunButton).unwrap();
    sm.process_event(Events::StartingFinished).unwrap();
    sm.process_event(Events::SetEmergency).unwrap();
    sm.process_event(Events::EmergencyOff).unwrap();
    sm.process_event(Events::FaultOn).unwrap();
    sm.process_event(Events::FaultOff).unwrap();
    sm.process_event(Events::MaintenanceButton).unwrap();
    sm.process_event(Events::MaintenanceButton).unwrap();
}
