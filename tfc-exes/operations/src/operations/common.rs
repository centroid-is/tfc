use serde::{Deserialize, Serialize};
use std::str::FromStr;

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

impl ToString for OperationMode {
    fn to_string(&self) -> String {
        match self {
            OperationMode::Unknown => "unknown".to_string(),
            OperationMode::Stopped => "stopped".to_string(),
            OperationMode::Starting => "starting".to_string(),
            OperationMode::Running => "running".to_string(),
            OperationMode::Stopping => "stopping".to_string(),
            OperationMode::Cleaning => "cleaning".to_string(),
            OperationMode::Emergency => "emergency".to_string(),
            OperationMode::Fault => "fault".to_string(),
            OperationMode::Maintenance => "maintenance".to_string(),
        }
    }
}

pub struct OperationsUpdate {
    pub new_mode: OperationMode,
    pub old_mode: OperationMode,
}

impl Default for OperationsUpdate {
    fn default() -> Self {
        Self {
            new_mode: OperationMode::Unknown,
            old_mode: OperationMode::Unknown,
        }
    }
}
