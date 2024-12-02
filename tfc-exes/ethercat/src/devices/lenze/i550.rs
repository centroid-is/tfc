use crate::devices::device_trait::Index;
use crate::devices::device_trait::WriteValueIndex;
use crate::devices::device_trait::{Device, DeviceInfo};
use async_trait::async_trait;
use atomic_refcell::AtomicRefMut;
use bitvec::view::BitView;
use ethercrab::EtherCrabWireSized;
use ethercrab::{SubDevice, SubDevicePdi, SubDeviceRef};
use ethercrab_wire::EtherCrabWireRead;
use ethercrab_wire::EtherCrabWireWrite;
use log::warn;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use tfc::confman::ConfMan;

use crate::define_value_type;
use crate::devices::CiA402;

static RX_PDO_ASSIGN: u16 = 0x1C12;
static TX_PDO_ASSIGN: u16 = 0x1C13;
static RX_PDO_MAPPING: u16 = 0x1605;
static TX_PDO_MAPPING: u16 = 0x1A05;
static BASIC_MOTOR_CONTROL: u16 = 0x2631;

#[derive(EtherCrabWireRead, PartialEq, Eq)]
#[repr(u16)]
enum I550Error {
    None = 0,
    ContinuousOverCurrent = 0x2250, // CiA: Continuous over current (internal) Fault -
    ShortCircuitInternal = 0x2320,  // CiA: Short circuit/earth leakage (internal) Fault -
    ShortCircuitDeviceInternal = 0x2340, // CiA: Short circuit (device internal) Fault -
    I2tOverload = 0x2350,           // CiA: i²*t overload (thermal state) Fault 0x2D4B:003 (P308.03)
    ItError = 0x2382,               // I*t error Fault 0x2D40:005 (P135.05)
    ItWarning = 0x2383,             // I*t warning Warning -
    ImaxClampRespondedTooOften = 0x2387, // Imax: Clamp responded too often Fault -
    StallDetectionActive = 0x2388,  // SL-PSM stall detection active Trouble -
    MaximumCurrentReached = 0x238A, // Maximum current reached Information -
    MainsPhaseFault = 0x3120,       // Mains phase fault Fault -
    UpsOperationActive = 0x3180,    // UPS operation active Warning -
    DcBusOvervoltage = 0x3210,      // DC bus overvoltage Fault -
    DcBusOvervoltageWarning = 0x3211, // DC bus overvoltage warning Warning -
    DcBusUndervoltage = 0x3220,     // DC bus undervoltage Trouble -
    DcBusUndervoltageWarning = 0x3221, // DC bus undervoltage warning Warning -
    DcBusVoltageTooLowForPowerUp = 0x3222, // DC-bus voltage to low for power up Warning -
    PowerUnitOvertemperature = 0x4210, // PU: overtemperature fault Fault -
    HeatSinkTemperatureSensorFault = 0x4280, // Heat sink temperature sensor fault Fault -
    HeatSinkFanWarning = 0x4281,    // Heat sink fan warning Warning -
    PowerUnitOvertemperatureWarning = 0x4285, // PU overtemperature warning Warning -
    MotorTemperatureError = 0x4310, // Motor temperature error Fault 0x2D49:002 (P309.02)
    TwentyFourVSupplyCritical = 0x5112, // 24 V supply critical Warning -
    OverloadTwentyFourVSupply = 0x5180, // Overload 24 V supply Warning -
    OemHardwareIncompatible = 0x5380, // OEM hardware incompatible Fault -
    InternalFanWarning = 0x618A,    // Internal fan warning Warning -
    TriggerFunctionsConnectedIncorrectly = 0x6280, // Trigger/functions connected incorrectly Trouble -
    UserDefinedFault1 = 0x6281,                    // User-defined fault 1 Fault -
    UserDefinedFault2 = 0x6282,                    // User-defined fault 2 Fault -
    WarningInvertRotation = 0x6290,                // Warning invert rotation Warning -
    MaximumAllowedTroublesExceeded = 0x6291,       // Maximuml allowed troubles exceeded Fault -
    UserDefinedFaultLecom = 0x62A0,                // User-defined fault (LECOM) Fault -
    NetworkUserFault1 = 0x62A1,                    // Network: user fault 1 Fault -
    NetworkUserFault2 = 0x62A2,                    // Network: user fault 2 Fault -
    NetworkWordIn1ConfigurationIncorrect = 0x62B1, // NetworkIN1 configuration incorrect Trouble -
    LoadErrorIdTag = 0x63A1,                       // CU: load error ID tag Fault -
    PuLoadErrorIdTag = 0x63A2,                     // PU: load error ID tag Fault -
    PowerUnitUnknown = 0x63A3,                     // Power unit unknown Fault -
    AssertionLevelMonitoring = 0x7080,             // Assertion level monitoring (Low/High) Fault -
    AnalogInput1Fault = 0x7081, // Analog input 1 fault Fault 0x2636:010 (P430.10)
    AnalogInput2Fault = 0x7082, // Analog input 2 fault Fault 0x2637:010 (P431.10)
    HtlInputFault = 0x7083,     // HTL input fault No response 0x2641:006 (P416.06)
    AnalogOutput1Fault = 0x70A1, // Analog output 1 fault Warning -
    AnalogOutput2Fault = 0x70A2, // Analog output 2 fault Warning -
    PolePositionIdentificationFault = 0x7121, // Pole position identification fault Fault 0x2C60
    MotorOvercurrent = 0x7180,  // Motor overcurrent Fault 0x2D46:002 (P353.02)
    EncoderOpenCircuit = 0x7305, // Encoder open circuit Warning 0x2C45 (P342.00)
    FeedbackSystemSpeedLimit = 0x7385, // Feedback system: speed limit Warning -
    MemoryModuleIsFull = 0x7680, // Memory module is full Warning -
    MemoryModuleNotPresent = 0x7681, // Memory module not present Fault
    MemoryModuleInvalidUserData = 0x7682, // Memory module: Invalid user data Fault -
    DataNotComplSavedBeforePowerdown = 0x7684, // Data not compl. saved before powerdown Warning -
    MemoryModuleInvalidOemData = 0x7689, // Memory module: invalid OEM data Warning -
    EpmFirmwareVersionIncompatible = 0x7690, // EPM firmware version incompatible Fault -
    EpmDataFirmwareTypeIncompatible = 0x7691, // EPM data: firmware type incompatible Fault -
    EpmDataNewFirmwareTypeDetected = 0x7692, // EPM data: new firmware type detected Fault -
    EpmDataPusizeIncompatible = 0x7693, // EPM data: PU size incompatible Fault -
    EpmDataNewPusizeDetected = 0x7694, // EPM data: new PU size detected Fault -
    InvalidParameterChangeoverConfiguration = 0x7695, // Invalid parameter changeover configuration Warning -
    EpmDataUnknownParameterFound = 0x7696, // EPM data: unknown parameter found Information -
    ParameterChangesLost = 0x7697,         // Parameter changes lost Fault -
    NetworkTimeoutExplicitMessage = 0x8112, // Network: timeout explicit message Warning 0x2859:006 (P515.06)
    NetworkOverallCommunicationTimeout = 0x8114, // Network: overall communication timeout Warning See details for 33044
    TimeOutPAM = 0x8115,                         // Time-out (PAM) No response 0x2552:004 (P595.04)
    ModbusTcpMasterTimeOut = 0x8116, // Modbus TCP master time-out Fault 0x2859:008 (P515.08)
    ModbusTcpKeepAliveTimeOut = 0x8117, // Modbus TCP Keep Alive time-out Fault 0x2859:009 (P515.09)
    CanBusOff = 0x8182,              // CAN: bus off Trouble 0x2857:010
    CanWarning = 0x8183,             // CAN: warning Warning 0x2857:011
    CanHeartbeatTimeOutConsumer1 = 0x8184, // CAN: heartbeat time-out consumer 1 Fault 0x2857:005
    CanHeartbeatTimeOutConsumer2 = 0x8185, // CAN: heartbeat time-out consumer 2 Fault 0x2857:006
    CanHeartbeatTimeOutConsumer3 = 0x8186, // CAN: heartbeat time-out consumer 3 Fault 0x2857:007
    CanHeartbeatTimeOutConsumer4 = 0x8187, // CAN: heartbeat time-out consumer 4 Fault 0x2857:008
    NetworkWatchdogTimeout = 0x8190, // Network: watchdog timeout Trouble See details for 33168
    NetworkDisruptionOfCyclicDataExchange = 0x8191, // Network: disruption of cyclic data exchange No response 0x2859:002 (P515.02)
    NetworkInitialisationError = 0x8192, // Network: initialisation error Trouble See details for 33170
    NetworkInvalidCyclicProcessData = 0x8193, // Network: invalid cyclic process data Trouble See details for 33171
    ModbusNetworkTimeOut = 0x81A1,            // Modbus: network time-out Fault 0x2858:001 (P515.01)
    ModbusIncorrectRequestByMaster = 0x81A2,  // Modbus: incorrect request by master Warning -
    NetworkCommunicationFaulty = 0x81B0,      // Network communication faulty Trouble -
    PowerlinkLossOfSoc = 0x8265,              // POWERLINK: Loss of SoC Trouble 0x2859:011
    PowerlinkCrcError = 0x8266,               // POWERLINK: CRC error Trouble 0x2859:010
    NetworkPdoMappingError = 0x8286, // Network: PDO mapping error Trouble See details for 33414
    CanRpdo1TimeOut = 0x8291,        // CAN: RPDO1 time-out Fault 0x2857:001
    CanRpdo2TimeOut = 0x8292,        // CAN: RPDO2 time-out Fault 0x2857:002
    CanRpdo3TimeOut = 0x8293,        // CAN: RPDO3 time-out Fault 0x2857:003
    TorqueLimitReached = 0x8311,     // Torque limit reached No response 0x2D67:001 (P329.01)
    FunctionNotAllowedInSelectedOperatingMode = 0x8380, // Function not allowed in selected operating mode Warning -
    KeypadRemoved = 0x9080,                             // Keypad removed Fault -
    BrakeResistorOverloadFault = 0xFF02, // Brake resistor: overload fault Fault 0x2550:011 (P707.11)
    SafeTorqueOffIsLocked = 0xFF05,      // Safe Torque Off is locked Fault -
    MotorOverspeed = 0xFF06,             // Motor overspeed Fault 0x2D44:002 (P350.02)
    MotorPhaseMissing = 0xFF09,          // Motor phase missing No response 0x2D45:001 (P310.01)
    MotorPhaseFailurePhaseU = 0xFF0A, // Motor phase failure phase U No response 0x2D45:001 (P310.01)
    MotorPhaseFailurePhaseV = 0xFF0B, // Motor phase failure phase V No response 0x2D45:001 (P310.01)
    MotorPhaseFailurePhaseW = 0xFF0C, // Motor phase failure phase W No response 0x2D45:001 (P310.01)
    MotorParameterIdentificationFault = 0xFF19, // Motor parameter identification fault Fault -
    FmfError = 0xFF1F,                // FMF Error Fault -
    BrakeResistorOverloadWarning = 0xFF36, // Brake resistor: overload warning Warning 0x2550:010 (P707.10)
    AutomaticStartDisabled = 0xFF37,       // Automatic start disabled Fault -
    LoadLossDetected = 0xFF38,             // Load loss detected No response 0x4006:003 (P710.03)
    MotorOverload = 0xFF39,                // Motor overload No response 0x4007:003
    MaximumMotorFrequencyReached = 0xFF56, // Maximum motor frequency reached Warning -
    ManualModeDeactivated = 0xFF5A,        // Manual mode deactivated Warning -
    ManualModeActivated = 0xFF5B,          // Manual mode activated Warning
    ManualModeTimeOut = 0xFF5C,            // Manual mode time-out Fault -
    WrongPassword = 0xFF71,                // Wrong password Warning -
    Warning = 0xFF72,                      // Warning Warning -
    FatalError = 0xFF73,                   // Fatal Error Fault -
    PowerUnitFatalError = 0xFF74,          // Power unit fatal error Fault -
    KeypadFullControlActive = 0xFF85,      // Keypad full control active Warning -
    AutoTuningActive = 0xFF86,             // Auto-tuning active Warning -
    AutoTuningCancelled = 0xFF87,          // Auto-tuning cancelled Fault
}
impl std::fmt::Debug for I550Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::ContinuousOverCurrent => {
                write!(f, "CiA: Continuous over current (internal) Fault")
            }
            Self::ShortCircuitInternal => {
                write!(f, "CiA: Short circuit/earth leakage (internal) Fault")
            }
            Self::ShortCircuitDeviceInternal => {
                write!(f, "CiA: Short circuit (device internal) Fault")
            }
            Self::I2tOverload => {
                write!(
                    f,
                    "CiA: i²*t overload (thermal state) Fault 0x2D4B:003 (P308.03)"
                )
            }
            Self::ItError => {
                write!(f, "I*t error Fault 0x2D40:005 (P135.05)")
            }
            Self::ItWarning => {
                write!(f, "I*t warning Warning")
            }
            Self::ImaxClampRespondedTooOften => {
                write!(f, "Imax: Clamp responded too often Fault")
            }
            Self::StallDetectionActive => {
                write!(f, "SL-PSM stall detection active Trouble")
            }
            Self::MaximumCurrentReached => {
                write!(f, "Maximum current reached Information")
            }
            Self::MainsPhaseFault => {
                write!(f, "Mains phase fault Fault")
            }
            Self::UpsOperationActive => {
                write!(f, "UPS operation active Warning")
            }
            Self::DcBusOvervoltage => {
                write!(f, "DC bus overvoltage Fault")
            }
            Self::DcBusOvervoltageWarning => {
                write!(f, "DC bus overvoltage warning Warning")
            }
            Self::DcBusUndervoltage => {
                write!(f, "DC bus undervoltage Trouble")
            }
            Self::DcBusUndervoltageWarning => {
                write!(f, "DC bus undervoltage warning Warning")
            }
            Self::DcBusVoltageTooLowForPowerUp => {
                write!(f, "DC-bus voltage too low for power up Warning")
            }
            Self::PowerUnitOvertemperature => {
                write!(f, "PU: overtemperature fault Fault")
            }
            Self::HeatSinkTemperatureSensorFault => {
                write!(f, "Heat sink temperature sensor fault Fault")
            }
            Self::HeatSinkFanWarning => {
                write!(f, "Heat sink fan warning Warning")
            }
            Self::PowerUnitOvertemperatureWarning => {
                write!(f, "PU overtemperature warning Warning")
            }
            Self::MotorTemperatureError => {
                write!(f, "Motor temperature error Fault 0x2D49:002 (P309.02)")
            }
            Self::TwentyFourVSupplyCritical => {
                write!(f, "24 V supply critical Warning")
            }
            Self::OverloadTwentyFourVSupply => {
                write!(f, "Overload 24 V supply Warning")
            }
            Self::OemHardwareIncompatible => {
                write!(f, "OEM hardware incompatible Fault")
            }
            Self::InternalFanWarning => {
                write!(f, "Internal fan warning Warning")
            }
            Self::TriggerFunctionsConnectedIncorrectly => {
                write!(f, "Trigger/functions connected incorrectly Trouble")
            }
            Self::UserDefinedFault1 => {
                write!(f, "User-defined fault 1 Fault")
            }
            Self::UserDefinedFault2 => {
                write!(f, "User-defined fault 2 Fault")
            }
            Self::WarningInvertRotation => {
                write!(f, "Warning invert rotation Warning")
            }
            Self::MaximumAllowedTroublesExceeded => {
                write!(f, "Maximum allowed troubles exceeded Fault")
            }
            Self::UserDefinedFaultLecom => {
                write!(f, "User-defined fault (LECOM) Fault")
            }
            Self::NetworkUserFault1 => {
                write!(f, "Network: user fault 1 Fault")
            }
            Self::NetworkUserFault2 => {
                write!(f, "Network: user fault 2 Fault")
            }
            Self::NetworkWordIn1ConfigurationIncorrect => {
                write!(f, "Network: IN1 configuration incorrect Trouble")
            }
            Self::LoadErrorIdTag => {
                write!(f, "CU: load error ID tag Fault")
            }
            Self::PuLoadErrorIdTag => {
                write!(f, "PU: load error ID tag Fault")
            }
            Self::PowerUnitUnknown => {
                write!(f, "Power unit unknown Fault")
            }
            Self::AssertionLevelMonitoring => {
                write!(f, "Assertion level monitoring (Low/High) Fault")
            }
            Self::AnalogInput1Fault => {
                write!(f, "Analog input 1 fault Fault 0x2636:010 (P430.10)")
            }
            Self::AnalogInput2Fault => {
                write!(f, "Analog input 2 fault Fault 0x2637:010 (P431.10)")
            }
            Self::HtlInputFault => {
                write!(f, "HTL input fault No response 0x2641:006 (P416.06)")
            }
            Self::AnalogOutput1Fault => {
                write!(f, "Analog output 1 fault Warning")
            }
            Self::AnalogOutput2Fault => {
                write!(f, "Analog output 2 fault Warning")
            }
            Self::PolePositionIdentificationFault => {
                write!(f, "Pole position identification fault Fault 0x2C60")
            }
            Self::MotorOvercurrent => {
                write!(f, "Motor overcurrent Fault 0x2D46:002 (P353.02)")
            }
            Self::EncoderOpenCircuit => {
                write!(f, "Encoder open circuit Warning 0x2C45 (P342.00)")
            }
            Self::FeedbackSystemSpeedLimit => {
                write!(f, "Feedback system: speed limit Warning")
            }
            Self::MemoryModuleIsFull => {
                write!(f, "Memory module is full Warning")
            }
            Self::MemoryModuleNotPresent => {
                write!(f, "Memory module not present Fault")
            }
            Self::MemoryModuleInvalidUserData => {
                write!(f, "Memory module: Invalid user data Fault")
            }
            Self::DataNotComplSavedBeforePowerdown => {
                write!(f, "Data not completely saved before powerdown Warning")
            }
            Self::MemoryModuleInvalidOemData => {
                write!(f, "Memory module: invalid OEM data Warning")
            }
            Self::EpmFirmwareVersionIncompatible => {
                write!(f, "EPM firmware version incompatible Fault")
            }
            Self::EpmDataFirmwareTypeIncompatible => {
                write!(f, "EPM data: firmware type incompatible Fault")
            }
            Self::EpmDataNewFirmwareTypeDetected => {
                write!(f, "EPM data: new firmware type detected Fault")
            }
            Self::EpmDataPusizeIncompatible => {
                write!(f, "EPM data: PU size incompatible Fault")
            }
            Self::EpmDataNewPusizeDetected => {
                write!(f, "EPM data: new PU size detected Fault")
            }
            Self::InvalidParameterChangeoverConfiguration => {
                write!(f, "Invalid parameter changeover configuration Warning")
            }
            Self::EpmDataUnknownParameterFound => {
                write!(f, "EPM data: unknown parameter found Information")
            }
            Self::ParameterChangesLost => {
                write!(f, "Parameter changes lost Fault")
            }
            Self::NetworkTimeoutExplicitMessage => {
                write!(
                    f,
                    "Network: timeout explicit message Warning 0x2859:006 (P515.06)"
                )
            }
            Self::NetworkOverallCommunicationTimeout => {
                write!(
                    f,
                    "Network: overall communication timeout Warning See details for 33044"
                )
            }
            Self::TimeOutPAM => {
                write!(f, "Time-out (PAM) No response 0x2552:004 (P595.04)")
            }
            Self::ModbusTcpMasterTimeOut => {
                write!(f, "Modbus TCP master time-out Fault 0x2859:008 (P515.08)")
            }
            Self::ModbusTcpKeepAliveTimeOut => {
                write!(
                    f,
                    "Modbus TCP Keep Alive time-out Fault 0x2859:009 (P515.09)"
                )
            }
            Self::CanBusOff => {
                write!(f, "CAN: bus off Trouble 0x2857:010")
            }
            Self::CanWarning => {
                write!(f, "CAN: warning Warning 0x2857:011")
            }
            Self::CanHeartbeatTimeOutConsumer1 => {
                write!(f, "CAN: heartbeat time-out consumer 1 Fault 0x2857:005")
            }
            Self::CanHeartbeatTimeOutConsumer2 => {
                write!(f, "CAN: heartbeat time-out consumer 2 Fault 0x2857:006")
            }
            Self::CanHeartbeatTimeOutConsumer3 => {
                write!(f, "CAN: heartbeat time-out consumer 3 Fault 0x2857:007")
            }
            Self::CanHeartbeatTimeOutConsumer4 => {
                write!(f, "CAN: heartbeat time-out consumer 4 Fault 0x2857:008")
            }
            Self::NetworkWatchdogTimeout => {
                write!(f, "Network: watchdog timeout Trouble See details for 33168")
            }
            Self::NetworkDisruptionOfCyclicDataExchange => {
                write!(
                    f,
                    "Network: disruption of cyclic data exchange No response 0x2859:002 (P515.02)"
                )
            }
            Self::NetworkInitialisationError => {
                write!(
                    f,
                    "Network: initialization error Trouble See details for 33170"
                )
            }
            Self::NetworkInvalidCyclicProcessData => {
                write!(
                    f,
                    "Network: invalid cyclic process data Trouble See details for 33171"
                )
            }
            Self::ModbusNetworkTimeOut => {
                write!(f, "Modbus: network time-out Fault 0x2858:001 (P515.01)")
            }
            Self::ModbusIncorrectRequestByMaster => {
                write!(f, "Modbus: incorrect request by master Warning")
            }
            Self::NetworkCommunicationFaulty => {
                write!(f, "Network communication faulty Trouble")
            }
            Self::PowerlinkLossOfSoc => {
                write!(f, "POWERLINK: Loss of SoC Trouble 0x2859:011")
            }
            Self::PowerlinkCrcError => {
                write!(f, "POWERLINK: CRC error Trouble 0x2859:010")
            }
            Self::NetworkPdoMappingError => {
                write!(
                    f,
                    "Network: PDO mapping error Trouble See details for 33414"
                )
            }
            Self::CanRpdo1TimeOut => {
                write!(f, "CAN: RPDO1 time-out Fault 0x2857:001")
            }
            Self::CanRpdo2TimeOut => {
                write!(f, "CAN: RPDO2 time-out Fault 0x2857:002")
            }
            Self::CanRpdo3TimeOut => {
                write!(f, "CAN: RPDO3 time-out Fault 0x2857:003")
            }
            Self::TorqueLimitReached => {
                write!(f, "Torque limit reached No response 0x2D67:001 (P329.01)")
            }
            Self::FunctionNotAllowedInSelectedOperatingMode => {
                write!(f, "Function not allowed in selected operating mode Warning")
            }
            Self::KeypadRemoved => {
                write!(f, "Keypad removed Fault")
            }
            Self::BrakeResistorOverloadFault => {
                write!(
                    f,
                    "Brake resistor: overload fault Fault 0x2550:011 (P707.11)"
                )
            }
            Self::SafeTorqueOffIsLocked => {
                write!(f, "Safe Torque Off is locked Fault")
            }
            Self::MotorOverspeed => {
                write!(f, "Motor overspeed Fault 0x2D44:002 (P350.02)")
            }
            Self::MotorPhaseMissing => {
                write!(f, "Motor phase missing No response 0x2D45:001 (P310.01)")
            }
            Self::MotorPhaseFailurePhaseU => {
                write!(
                    f,
                    "Motor phase failure phase U No response 0x2D45:001 (P310.01)"
                )
            }
            Self::MotorPhaseFailurePhaseV => {
                write!(
                    f,
                    "Motor phase failure phase V No response 0x2D45:001 (P310.01)"
                )
            }
            Self::MotorPhaseFailurePhaseW => {
                write!(
                    f,
                    "Motor phase failure phase W No response 0x2D45:001 (P310.01)"
                )
            }
            Self::MotorParameterIdentificationFault => {
                write!(f, "Motor parameter identification fault Fault")
            }
            Self::FmfError => {
                write!(f, "FMF Error Fault")
            }
            Self::BrakeResistorOverloadWarning => {
                write!(
                    f,
                    "Brake resistor: overload warning Warning 0x2550:010 (P707.10)"
                )
            }
            Self::AutomaticStartDisabled => {
                write!(f, "Automatic start disabled Fault")
            }
            Self::LoadLossDetected => {
                write!(f, "Load loss detected No response 0x4006:003 (P710.03)")
            }
            Self::MotorOverload => {
                write!(f, "Motor overload No response 0x4007:003")
            }
            Self::MaximumMotorFrequencyReached => {
                write!(f, "Maximum motor frequency reached Warning")
            }
            Self::ManualModeDeactivated => {
                write!(f, "Manual mode deactivated Warning")
            }
            Self::ManualModeActivated => {
                write!(f, "Manual mode activated Warning")
            }
            Self::ManualModeTimeOut => {
                write!(f, "Manual mode time-out Fault")
            }
            Self::WrongPassword => {
                write!(f, "Wrong password Warning")
            }
            Self::Warning => {
                write!(f, "Warning Warning 0xFF72")
            }
            Self::FatalError => {
                write!(f, "Fatal Error Fault")
            }
            Self::PowerUnitFatalError => {
                write!(f, "Power unit fatal error Fault")
            }
            Self::KeypadFullControlActive => {
                write!(f, "Keypad full control active Warning")
            }
            Self::AutoTuningActive => {
                write!(f, "Auto-tuning active Warning")
            }
            Self::AutoTuningCancelled => {
                write!(f, "Auto-tuning cancelled Fault")
            }
        }
    }
}

impl std::fmt::Display for I550Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(ethercrab_wire::EtherCrabWireRead, Debug)]
#[wire(bytes = 4)]
struct Inputs {
    #[wire(bits = 16)]
    _unused1: u16,
    #[wire(bits = 7)]
    inputs: u8,
    // 0 = digital input terminals are set to HIGH (PNP) level via pull-up resistors.
    // 1 = digital input terminals are set to LOW (PNP) level via pull-down resistors.
    #[wire(bits = 1)]
    interconnection: bool, // Internal interconnection of digital inputs
    #[wire(bits = 8)]
    _unused2: u8,
}

#[derive(ethercrab_wire::EtherCrabWireWrite, Debug)]
#[wire(bytes = 4)]
struct OutputPdo {
    #[wire(bits = 16)]
    control_word: CiA402::ControlWord,
    // control_word: ethercrab::ds402::ControlWord,
    #[wire(bits = 16)]
    speed: i16,
    // #[wire(bits = 8)]
    // output1: bool,
}

#[derive(ethercrab_wire::EtherCrabWireRead, Debug)]
#[wire(bytes = 16)]
struct InputPdo {
    #[wire(bits = 16)]
    status_word: CiA402::StatusWord,
    #[wire(bits = 16)]
    actual_speed: i16,
    #[wire(bits = 16)]
    error: I550Error,
    #[wire(bits = 16)]
    current: i16, // deciampere
    #[wire(bits = 16)]
    frequency: i16, // decihertz
    #[wire(bits = 32)]
    inputs: Inputs,
    #[wire(bits = 16)]
    analog_input_1: u16,
}

#[derive(Debug, Copy, Clone, EtherCrabWireWrite, Serialize, Deserialize, JsonSchema)]
#[repr(u8)]
enum RatedMainsVoltage {
    Veff230 = 0,
    Veff400 = 1,
    Veff480 = 2,
    Veff120 = 3,
    Veff230ReducedLuLevel = 10,
}
impl Default for RatedMainsVoltage {
    fn default() -> Self {
        Self::Veff400
    }
}
impl Index for RatedMainsVoltage {
    const INDEX: u16 = 0x2540;
    const SUBINDEX: u8 = 0x01;
}

#[derive(Debug, Copy, Clone, EtherCrabWireWrite, Serialize, Deserialize, JsonSchema)]
#[repr(u8)]
enum AnalogInput1 {
    VDC0To10 = 0,
    VDC0To5 = 1,
    VDC2To10 = 2,
    VDCNeg10ToPos10 = 3,
    #[allow(non_camel_case_types)]
    mA4To20 = 4,
    #[allow(non_camel_case_types)]
    mA0To20 = 5,
}
impl Default for AnalogInput1 {
    fn default() -> Self {
        Self::mA4To20
    }
}
impl Index for AnalogInput1 {
    const INDEX: u16 = 0x2636;
    const SUBINDEX: u8 = 0x01;
}

#[derive(EtherCrabWireRead, PartialEq, Eq, Debug)]
#[wire(bytes = 2)]
struct AnalogInput1Fault {
    #[wire(bits = 1)]
    vdc0_to_10: bool,
    #[wire(bits = 1)]
    vdc0_to_5: bool,
    #[wire(bits = 1)]
    vdc2_to_10: bool,
    #[wire(bits = 1)]
    vdcneg10_to_pos10: bool,
    #[wire(bits = 1)]
    mA4_to_20: bool,
    #[wire(bits = 1)]
    mA0_to_20: bool,
    #[wire(bits = 1)]
    supply24v_ok: bool,
    #[wire(bits = 1)]
    calibration_successful: bool,
    #[wire(bits = 1)]
    monitor_threshold_exceeded: bool,
    #[wire(bits = 1)]
    input_current_too_low: bool,
    #[wire(bits = 1)]
    input_voltage_too_low: bool,
    #[wire(bits = 1, post_skip = 4)]
    input_voltage_too_high: bool,
}
impl Index for AnalogInput1Fault {
    const INDEX: u16 = 0x2DA4;
    const SUBINDEX: u8 = 16;
}

define_value_type!(BaseVoltage, u16, 400, 0x2B01, 1); // volts
define_value_type!(BaseFrequency, u16, 50, 0x2B01, 2); // hertz
define_value_type!(MaxSpeed, u32, 6075, 0x6080, 0); // rpm
define_value_type!(MinSpeed, u32, 0, 0x6046, 1); // rpm
define_value_type!(AccelerationNumerator, u32, 3000, 0x6048, 1); // rpm
define_value_type!(AccelerationDenominator, u16, 10, 0x6048, 2); // seconds
                                                                 // btw this is rpm/s both acceleration and deceleration
define_value_type!(DecelerationNumerator, u32, 3000, 0x6049, 1); // rpm
define_value_type!(DecelerationDenominator, u16, 10, 0x6049, 2); // seconds
define_value_type!(StatorResistance, u32, 101565, 0x2C01, 2); // 10.1565 ohms so factor 10000
define_value_type!(StatorLeakageInductance, u32, 23566, 0x2C01, 3); // 23.566 mH so factor 1000
define_value_type!(RatedSpeed, u16, 1450, 0x2C01, 4); // rpm
define_value_type!(RatedFrequency, u16, 500, 0x2C01, 5); // decihertz
define_value_type!(RatedPower, u16, 750, 0x2C01, 6); // centiwatts
define_value_type!(RatedVoltage, u16, 400, 0x2C01, 7); // volts
define_value_type!(RatedCurrent, u32, 1700, 0x6075, 0); // milliampere
define_value_type!(CosinePhi, u16, 80, 0x2C01, 8); // cosine phi factor 100
define_value_type!(MaxCurrent, u16, 2000, 0x6073, 0); // percent factor 10 2000 is 200.0%

// todo rotor time constant

#[derive(Serialize, Deserialize, JsonSchema, Debug, Default)]
struct Acceleration {
    #[schemars(
        description = "Acceleration numerator in RPM",
        range(min = 0, max = 2147483647)
    )]
    numerator: AccelerationNumerator,
    #[schemars(
        description = "Acceleration denominator in seconds",
        range(min = 0, max = 65535)
    )]
    denominator: AccelerationDenominator,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Default)]
struct Deceleration {
    #[schemars(
        description = "Deceleration numerator in RPM",
        range(min = 0, max = 2147483647)
    )]
    numerator: DecelerationNumerator,
    #[schemars(
        description = "Deceleration denominator in seconds",
        range(min = 0, max = 65535) // todo min 1 right?
    )]
    denominator: DecelerationDenominator,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Default)]
struct Config {
    // lenze i550 manual 5.8.2 Manual setting of the motor data
    // todo: add the other parameters
    rated_mains_voltage: RatedMainsVoltage,
    #[schemars(description = "Base voltage in volts", range(min = 0, max = 5000))]
    base_voltage: BaseVoltage,
    #[schemars(description = "Base frequency in hertz", range(min = 0, max = 1500))]
    base_frequency: BaseFrequency,
    #[schemars(description = "Max speed in RPM", range(min = 0, max = 480000))]
    max_speed: MaxSpeed,
    #[schemars(description = "Min speed in RPM", range(min = 0, max = 480000))]
    min_speed: MinSpeed,
    #[schemars(description = "Acceleration in RPM/s")]
    acceleration: Acceleration,
    #[schemars(description = "Deceleration in RPM/s")]
    deceleration: Deceleration,
    #[schemars(
        description = "Stator resistance in factor of 10000 ohms, 101565 is 10.1565 ohms. Typical for 120Hz Lenze = 1100mOhm"
    )]
    stator_resistance: Option<StatorResistance>,
    #[schemars(
        description = "Stator leakage inductance in factor of 1000 mH, 23566 is 23.566 mH. Typical for 120Hz Lenze = 61mH"
    )]
    stator_leakage_inductance: Option<StatorLeakageInductance>,
    #[schemars(description = "Rated speed in RPM")]
    rated_speed: RatedSpeed,
    #[schemars(description = "Rated frequency in decihertz")]
    rated_frequency: RatedFrequency,
    #[schemars(description = "Rated power in centiwatts")]
    rated_power: RatedPower,
    #[schemars(description = "Rated voltage in volts")]
    rated_voltage: RatedVoltage,
    #[schemars(description = "Rated current in milliampere")]
    rated_current: RatedCurrent,
    #[schemars(description = "Cosine phi factor of 100")]
    cosine_phi: CosinePhi,
    #[schemars(description = "Max current in percent of rated current, 2000 is 200.0%")]
    max_current: MaxCurrent,
    #[schemars(description = "Analog input 1 mode")]
    analog_input_1: AnalogInput1,
    #[schemars(description = "Default speed ratio, -100.0% to 100.0%")]
    speedratio: f32,
}

pub struct I550 {
    cnt: u128,
    config: ConfMan<Config>,
    inputs: [tfc::ipc::Signal<bool>; 7],
    last_inputs: [Option<bool>; 7],
    speedratio: tfc::ipc::Slot<f64>,
    rpm_setpoint: Arc<std::sync::atomic::AtomicI16>,
    run: tfc::ipc::Slot<bool>,
    run_cached: Arc<std::sync::atomic::AtomicBool>,
    log_key: String,
    speedratio_handle: tokio::task::JoinHandle<()>,
    run_handle: tokio::task::JoinHandle<()>,
}

fn map(value: f64, in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> f64 {
    let clamped_value = value.clamp(in_min, in_max);
    (clamped_value - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
}
fn percentage_to_rpm(percentage: f64, min_rpm: MinSpeed, max_rpm: MaxSpeed) -> i16 {
    if percentage.abs() < 1.0 {
        return 0;
    }

    let mapped = map(
        percentage.abs(),
        1.0,
        100.0,
        min_rpm.value as f64,
        max_rpm.value as f64,
    );

    if percentage <= -1.0 {
        return (-1.0 * mapped) as i16;
    }
    mapped as i16
}

impl I550 {
    pub fn new(dbus: zbus::Connection, sub_number: u16, alias_address: u16) -> Self {
        let mut prefix = format!("i550/{sub_number}");
        if alias_address != 0 {
            prefix = format!("i550/alias/{alias_address}");
        }
        let config: ConfMan<Config> = ConfMan::new(dbus.clone(), &prefix);
        let speedratio: tfc::ipc::Slot<f64> = tfc::ipc::Slot::new(
            dbus.clone(),
            tfc::ipc::Base::new(format!("{prefix}/speedratio").as_str(), None),
        );
        #[cfg(feature = "dbus-expose")]
        tfc::ipc::dbus::SlotInterface::register(
            speedratio.base(),
            dbus.clone(),
            speedratio.channel("dbus"),
        );
        let mut speedratio_channel = speedratio.subscribe();
        let min_speed = config.read().min_speed; // todo this is not updatable ...
        let max_speed = config.read().max_speed;
        let rpm_setpoint = Arc::new(std::sync::atomic::AtomicI16::new(percentage_to_rpm(
            config.read().speedratio as f64,
            min_speed,
            max_speed,
        )));
        let rpm_setpoint_cp = Arc::clone(&rpm_setpoint);
        let log_key_cp = prefix.clone();
        let speedratio_handle = tokio::spawn(async move {
            while let Ok(()) = speedratio_channel.changed().await {
                let speedratio = *speedratio_channel.borrow_and_update();
                if let Some(speedratio) = speedratio {
                    rpm_setpoint_cp.store(
                        percentage_to_rpm(speedratio, min_speed, max_speed),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
            }
            warn!(target: &log_key_cp, "speedratio channel closed");
        });

        let run = tfc::ipc::Slot::new(
            dbus.clone(),
            tfc::ipc::Base::new(format!("{prefix}/run").as_str(), None),
        );
        #[cfg(feature = "dbus-expose")]
        tfc::ipc::dbus::SlotInterface::register(run.base(), dbus.clone(), run.channel("dbus"));
        let run_cached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let run_cached_cp = Arc::clone(&run_cached);
        let mut run_channel = run.subscribe();
        let log_key_cp = prefix.clone();
        let run_handle = tokio::spawn(async move {
            while let Ok(()) = run_channel.changed().await {
                if let Some(run) = *run_channel.borrow_and_update() {
                    run_cached_cp.store(run, std::sync::atomic::Ordering::Relaxed);
                }
            }
            warn!(target: &log_key_cp, "run channel closed");
        });

        Self {
            cnt: 0,
            config,
            inputs: std::array::from_fn(|idx| {
                let signal = tfc::ipc::Signal::new(
                    dbus.clone(),
                    tfc::ipc::Base::new(format!("{prefix}/DI{}", idx + 1).as_str(), None),
                );
                #[cfg(feature = "dbus-expose")]
                tfc::ipc::dbus::SignalInterface::register(
                    signal.base(),
                    dbus.clone(),
                    signal.subscribe(),
                );
                signal
            }),
            last_inputs: [None; 7],
            speedratio,
            rpm_setpoint,
            run,
            run_cached,
            log_key: prefix,
            speedratio_handle,
            run_handle,
        }
    }
}

impl Drop for I550 {
    fn drop(&mut self) {
        self.speedratio_handle.abort();
        self.run_handle.abort();
    }
}

#[async_trait]
impl Device for I550 {
    async fn setup<'maindevice, 'group>(
        &mut self,
        device: &mut SubDeviceRef<'maindevice, AtomicRefMut<'group, SubDevice>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        warn!("Setting up I550");

        // reset fault
        device.sdo_write(BASIC_MOTOR_CONTROL, 4, 1 as u8).await?;

        device.sdo_write(RX_PDO_ASSIGN, 0x00, 0 as u8).await?;
        device.sdo_write(TX_PDO_ASSIGN, 0x00, 0 as u8).await?;

        // zero the size
        device.sdo_write(RX_PDO_MAPPING, 0x00, 0 as u8).await?;

        device
            .sdo_write(RX_PDO_MAPPING, 0x01, 0x60400010 as u32)
            .await?; // CMD - Control Word
        device
            .sdo_write(RX_PDO_MAPPING, 0x02, 0x60420010 as u32)
            .await?; // set speed

        device.sdo_write(RX_PDO_MAPPING, 0x00, 2 as u8).await?;

        // zero the size
        device.sdo_write(TX_PDO_MAPPING, 0x00, 0 as u8).await?;

        device
            .sdo_write(TX_PDO_MAPPING, 0x01, 0x60410010 as u32)
            .await?; // ETA  - STATUS WORD
        device
            .sdo_write(TX_PDO_MAPPING, 0x02, 0x60440010 as u32)
            .await?; // Actual speed
        device
            .sdo_write(TX_PDO_MAPPING, 0x03, 0x603F0010 as u32)
            .await?; // Error
        device
            .sdo_write(TX_PDO_MAPPING, 0x04, 0x2d880010 as u32)
            .await?; // Actual current
        device
            .sdo_write(TX_PDO_MAPPING, 0x05, 0x2ddd0010 as u32)
            .await?; // Actual frequency

        device
            .sdo_write(TX_PDO_MAPPING, 0x06, 0x60FD0020 as u32) // address 0x60FD, subindex 0x00, length 0x20 = 32 bytes
            .await?; // Inputs

        device
            .sdo_write(TX_PDO_MAPPING, 0x07, 0x2DA40110 as u32)
            .await?; // Analog input 1

        // Set tx size
        device.sdo_write(TX_PDO_MAPPING, 0x00, 7 as u8).await?;

        // Assign pdo's to mappings
        device
            .sdo_write(RX_PDO_ASSIGN, 0x01, RX_PDO_MAPPING as u16)
            .await?;
        device.sdo_write(RX_PDO_ASSIGN, 0x00, 1 as u8).await?;

        device
            .sdo_write(TX_PDO_ASSIGN, 0x01, TX_PDO_MAPPING as u16)
            .await?;
        device.sdo_write(TX_PDO_ASSIGN, 0x00, 1 as u8).await?;

        // cia 402 velocity mode
        device.sdo_write(0x6060, 0, 2 as u8).await?;

        device.sdo_write(BASIC_MOTOR_CONTROL, 0x01, 1 as u8).await?; // Set enable inverter to true

        // device.sdo_write(BASIC_MOTOR_CONTROL, 0x02, 1 as u8).await?; // Set allow run to constant true

        device
            .sdo_write_value_index(self.config.read().rated_mains_voltage)
            .await?;
        device
            .sdo_write_value_index(self.config.read().base_voltage)
            .await?;
        device
            .sdo_write_value_index(self.config.read().base_frequency)
            .await?;
        device
            .sdo_write_value_index(self.config.read().max_speed)
            .await?;
        // I don't know why max speed is in two different places in i550
        device
            .sdo_write(0x6046, 2, self.config.read().max_speed.value)
            .await?;
        device
            .sdo_write_value_index(self.config.read().min_speed)
            .await?;
        device
            .sdo_write_value_index(self.config.read().acceleration.numerator)
            .await?;
        device
            .sdo_write_value_index(self.config.read().acceleration.denominator)
            .await?;
        if let Some(stator_resistance) = self.config.read().stator_resistance {
            device.sdo_write_value_index(stator_resistance).await?;
        }
        if let Some(stator_leakage_inductance) = self.config.read().stator_leakage_inductance {
            device
                .sdo_write_value_index(stator_leakage_inductance)
                .await?;
        }
        device
            .sdo_write_value_index(self.config.read().analog_input_1)
            .await?;
        // analog input 1 no response on error
        device.sdo_write(0x2636, 10, 0 as u8).await?;

        warn!("I550 setup complete");
        Ok(())
    }
    async fn process_data<'maindevice, 'group>(
        &mut self,
        device: &mut SubDeviceRef<'maindevice, SubDevicePdi<'group>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (input, output) = device.io_raw_mut();

        if output.len() != OutputPdo::PACKED_LEN {
            warn!(
                "Output PDO length is not {}, output length is {}",
                OutputPdo::PACKED_LEN,
                output.len()
            );
            return Ok(());
        }

        if input.len() != InputPdo::PACKED_LEN {
            warn!(
                "Input PDO length is not {}, input length is {}",
                InputPdo::PACKED_LEN,
                input.len()
            );
            return Ok(());
        }

        let input_pdo = InputPdo::unpack_from_slice(&input).expect("Error unpacking input PDO");
        let current_state = input_pdo.status_word.parse_state();
        let auto_reset_allowed = true;

        let mut control_word = CiA402::transition(
            current_state.clone(),
            CiA402::TransitionAction::Stop,
            auto_reset_allowed,
        );
        let setpoint = self.rpm_setpoint.load(std::sync::atomic::Ordering::Relaxed);
        if setpoint != 0 && self.run_cached.load(std::sync::atomic::Ordering::Relaxed) {
            control_word = CiA402::transition(
                current_state,
                CiA402::TransitionAction::Run,
                auto_reset_allowed,
            );
        }

        let output_pdo = OutputPdo {
            control_word,
            speed: self.rpm_setpoint.load(std::sync::atomic::Ordering::Relaxed),
            // output1: false,
        };
        output_pdo
            .pack_to_slice(&mut *output)
            .expect("Error packing output PDO");

        self.cnt += 1;

        let input_bits = input_pdo.inputs.inputs.view_bits::<bitvec::order::Lsb0>();
        for idx in 0..7 {
            let bit = input_bits[idx];
            if self.last_inputs[idx].is_none() || self.last_inputs[idx].unwrap() != bit {
                let _ = self.inputs[idx].async_send(bit).await.map_err(|e| {
                    warn!(target: &self.log_key, "Error sending signal: {e}");
                    e
                });
            }
            self.last_inputs[idx] = Some(bit);
        }

        if self.cnt % 10000 == 0 {
            warn!("output_pdo: {:?}", output_pdo);
            warn!("input_pdo: {:?}", input_pdo);
            warn!("output: {:?}", output);
            // if input_pdo.error == I550Error::AnalogInput1Fault {
            //     let fault: [u8; 2] = device
            //         .sdo_read(AnalogInput1Fault::INDEX, AnalogInput1Fault::SUBINDEX)
            //         .await?;
            //     let fault = AnalogInput1Fault::unpack_from_slice(&fault)
            //         .expect("Error unpacking analog input 1 fault");
            //     warn!("Analog input 1 fault: {:?}", fault);
            // }
        }

        Ok(())
    }
    fn vendor_id(&self) -> u32 {
        Self::VENDOR_ID
    }
    fn product_id(&self) -> u32 {
        Self::PRODUCT_ID
    }
}

impl DeviceInfo for I550 {
    const VENDOR_ID: u32 = 0x0000003b;
    const PRODUCT_ID: u32 = 0x69055000;
    const NAME: &'static str = "i550";
}

#[cfg(test)]
mod tests {
    use super::*;

    use uom::si::electric_potential::decivolt;
    use uom::si::length::centimeter;
    // mod cgs {
    //     use uom::system;
    //     uom::ISQ!(
    //         uom::si,
    //         f32,
    //         (centimeter, gram, second, ampere, kelvin, mole, candela)
    //     );
    // }
    // mod foo {
    //     use uom::system;
    //     uom::ISQ!(uom::si, i16, (decivolt));
    // }

    #[test]
    fn test_base_voltage() {
        // assert_eq!(base_voltage.value, 4000);

        // // store base voltage quantity in decivolt
        let base_voltage =
            uom::si::i16::ElectricPotential::new::<uom::si::electric_potential::decivolt>(4001);
        let value_in_decivolts = base_voltage.get::<uom::si::electric_potential::decivolt>();
        assert_eq!(value_in_decivolts, 4001);
        assert_eq!(base_voltage.value, 4000);
    }
}
