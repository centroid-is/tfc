use crate::devices::device_trait::Index;
use ethercrab_wire::{EtherCrabWireRead, EtherCrabWireWrite};

#[derive(Default, Debug, EtherCrabWireWrite)]
#[wire(bytes = 2)]
pub struct ControlWord {
    #[wire(bits = 1)]
    switch_on: bool,
    #[wire(bits = 1)]
    enable_voltage: bool,
    #[wire(bits = 1)]
    quick_stop: bool, // FYI, enabled low
    #[wire(bits = 1)]
    enable_op: bool,
    #[wire(bits = 1)]
    op_specific_1: bool,
    #[wire(bits = 1)]
    op_specific_2: bool,
    #[wire(bits = 1)]
    op_specific_3: bool,
    #[wire(bits = 1)]
    reset_fault: bool,
    #[wire(bits = 1)]
    halt: bool,
    #[wire(bits = 1)]
    op_specific_4: bool,
    #[wire(bits = 1)]
    bit_10: bool,
    #[wire(bits = 1)]
    bit_11: bool,
    #[wire(bits = 1)]
    bit_12: bool,
    #[wire(bits = 1)]
    bit_13: bool,
    #[wire(bits = 1)]
    bit_14: bool,
    #[wire(bits = 1)]
    bit_15: bool,
}

impl Index for ControlWord {
    const INDEX: u16 = 0x6040;
    const SUBINDEX: u8 = 0x00;
}

impl ControlWord {
    fn state_shutdown() -> Self {
        Self {
            switch_on: false,
            enable_voltage: true,
            quick_stop: true,
            enable_op: false,
            ..Default::default()
        }
    }
    fn state_switch_on() -> Self {
        Self {
            switch_on: true,
            enable_voltage: true,
            quick_stop: true,
            ..Default::default()
        }
    }
    fn state_enable_operation() -> Self {
        Self {
            switch_on: true,
            enable_voltage: true,
            quick_stop: true,
            enable_op: true,
            ..Default::default()
        }
    }
    fn state_disable_operation() -> Self {
        Self {
            switch_on: true,
            enable_voltage: true,
            quick_stop: true,
            enable_op: false,
            ..Default::default()
        }
    }
    fn disable_voltage() -> Self {
        Self {
            ..Default::default()
        }
    }
    fn quick_stop() -> Self {
        Self {
            enable_voltage: true,
            ..Default::default()
        }
    }
    fn fault_reset() -> Self {
        Self {
            reset_fault: true,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
pub enum State {
    NotReadyToSwitchOn = 1,
    SwitchOnDisabled = 2,
    ReadyToSwitchOn = 3,
    SwitchedOn = 4,
    OperationEnabled = 5,
    QuickStopActive = 6,
    FaultReactionActive = 7,
    Fault = 8,
}

#[derive(Default, Debug, EtherCrabWireRead)]
#[wire(bytes = 2)]
pub struct StatusWord {
    #[wire(bits = 1)]
    state_ready_to_switch_on: bool,
    #[wire(bits = 1)]
    state_switched_on: bool,
    #[wire(bits = 1)]
    state_operation_enabled: bool,
    #[wire(bits = 1)]
    state_fault: bool,
    #[wire(bits = 1)]
    voltage_enabled: bool,
    #[wire(bits = 1)]
    state_quick_stop: bool,
    #[wire(bits = 1)]
    state_switch_on_disabled: bool,
    #[wire(bits = 1)]
    warning: bool,
    #[wire(bits = 1)]
    halt_request_active: bool,
    #[wire(bits = 1)]
    remote: bool,
    #[wire(bits = 1)]
    target_reached: bool,
    #[wire(bits = 1)]
    internal_limit_active: bool,
    #[wire(bits = 1)]
    application_specific_1: bool,
    #[wire(bits = 1)]
    application_specific_2: bool,
    #[wire(bits = 1)]
    application_specific_3: bool,
    #[wire(bits = 1)]
    application_specific_4: bool,
}

impl Index for StatusWord {
    const INDEX: u16 = 0x6041;
    const SUBINDEX: u8 = 0x00;
}

impl StatusWord {
    pub fn parse_state(&self) -> State {
        /*
          Status Word Bit Mapping:
          Bit 0: Ready to switch on.
          Bit 1: Switched on.
          Bit 2: Operation enabled.
          Bit 3: Fault.
          Bit 4: Voltage enabled.
          Bit 5: Quick stop. (FYI, enabled low)
          Bit 6: Switch on disabled.

          Bit 6 | Bit 5 | Bit 4 | Bit 3 | Bit 2 | Bit 1 | Bit 0 | State
          ------------------------------------------------------------
            0   |   x   |   x   |   0   |   0   |   0   |   0   | State 1: Not ready to switch on
            1   |   x   |   x   |   0   |   0   |   0   |   0   | State 2: Switch on disabled
            0   |   1   |   x   |   0   |   0   |   0   |   1   | State 3: Ready to switch on
            0   |   1   |   1   |   0   |   0   |   1   |   1   | State 4: Switched on
            0   |   1   |   1   |   0   |   1   |   1   |   1   | State 5: Operation enabled
            0   |   0   |   1   |   0   |   1   |   1   |   1   | State 6: Quick stop active
            0   |   x   |   x   |   1   |   1   |   1   |   1   | State 7: Fault reaction active
            0   |   x   |   x   |   1   |   0   |   0   |   0   | State 8: Fault
        */
        if self.state_fault {
            if self.state_operation_enabled
                && self.state_switched_on
                && self.state_ready_to_switch_on
            {
                return State::FaultReactionActive;
            }
            return State::Fault;
        }
        if !self.state_ready_to_switch_on
            && !self.state_switched_on
            && !self.state_operation_enabled
            && !self.state_switch_on_disabled
        {
            return State::NotReadyToSwitchOn;
        }
        if self.state_switch_on_disabled {
            return State::SwitchOnDisabled;
        }
        if !self.state_quick_stop {
            return State::QuickStopActive;
        }
        if self.state_ready_to_switch_on && self.state_quick_stop && !self.state_switched_on {
            return State::ReadyToSwitchOn;
        }
        if self.state_ready_to_switch_on && self.state_switched_on && self.voltage_enabled {
            if self.state_operation_enabled {
                return State::OperationEnabled;
            }
            return State::SwitchedOn;
        }
        return State::NotReadyToSwitchOn;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TransitionAction {
    None = 0,
    Run,
    Stop,
    // Note. For now this is not directly used but has the same effect as none as run is not set.
    QuickStop,
    FreewheelStop,
    Reset,
}

/**
 * Transition to operational mode
 * @param current_state State parsed from status word determined to be the current status of a drive
 * @param action the action to change to another state in cia402 state machine
 * @param auto_reset_allowed allowance flag to indicate whether it is okay to move from fault state
 * @return the command to transition to operational mode / stick in quick stop mode.
 */
pub fn transition(
    current_state: State,
    action: TransitionAction,
    auto_reset_allowed: bool,
) -> ControlWord {
    match current_state {
        State::SwitchOnDisabled => return ControlWord::state_shutdown(),
        State::SwitchedOn | State::ReadyToSwitchOn => {
            if action == TransitionAction::Run {
                return ControlWord::state_enable_operation(); // This is a shortcut marked as 3B in ethercat manual for atv320
            }
            return ControlWord::state_disable_operation(); // Stay in this state if in ready to switch on else transition to switched on
        }
        State::OperationEnabled => {
            if action == TransitionAction::QuickStop {
                return ControlWord::quick_stop();
            }
            if action == TransitionAction::FreewheelStop {
                return ControlWord::disable_voltage(); // Freewheel stop
            }
            if action != TransitionAction::Run {
                return ControlWord::state_disable_operation();
            }

            return ControlWord::state_enable_operation();
        }
        State::Fault => {
            if action == TransitionAction::Reset || auto_reset_allowed {
                return ControlWord::fault_reset();
            }
            // We are not allowed to reset the fault that has occured.
            return ControlWord::state_shutdown();
        }
        State::QuickStopActive => {
            if action == TransitionAction::QuickStop {
                return ControlWord::quick_stop();
            }
            return ControlWord::disable_voltage();
        }
        State::NotReadyToSwitchOn => {
            return ControlWord::state_shutdown();
        }
        State::FaultReactionActive => {
            return ControlWord::disable_voltage();
        }
    }
    // Can only occur if someone casts an integer for state_e that is not defined in the enum
    return ControlWord::disable_voltage();
}
