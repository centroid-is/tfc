use log::{self, Level, Log, Metadata, Record, SetLoggerError};
use std::sync::Once;
use syslog::{BasicLogger, Facility, Formatter3164};

use crate::progbase;

static INIT: Once = Once::new();

struct CombinedLogger {
    env_logger: env_logger::Logger,
    syslog_logger: BasicLogger,
}

impl Log for CombinedLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.env_logger.enabled(metadata) || self.syslog_logger.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if self.env_logger.enabled(record.metadata()) {
            self.env_logger.log(record);
        }
        if self.syslog_logger.enabled(record.metadata()) {
            self.syslog_logger.log(record);
        }
    }

    fn flush(&self) {
        self.env_logger.flush();
        self.syslog_logger.flush();
    }
}

fn init_combined_logger() -> Result<(), SetLoggerError> {
    INIT.call_once(|| {
        let env_logger: env_logger::Logger =
            env_logger::Builder::from_env(env_logger::Env::default())
                .filter_level(progbase::log_lvl())
                .build();

        let formatter = Formatter3164 {
            facility: Facility::LOG_USER,
            hostname: None,
            process: "rust-logger".into(),
            pid: 0,
        };

        let syslog_logger = match syslog::unix(formatter) {
            Ok(logger) => BasicLogger::new(logger),
            Err(e) => {
                eprintln!("Could not connect to syslog: {:?}", e);
                return;
            }
        };

        let combined_logger = CombinedLogger {
            env_logger,
            syslog_logger,
        };

        let _ = log::set_boxed_logger(Box::new(combined_logger))
            .map(|()| log::set_max_level(progbase::log_lvl()));
    });

    Ok(())
}
pub struct Logger {
    key: String,
}

impl Logger {
    pub fn new(key: &str) -> Self {
        let _ = init_combined_logger();
        Logger {
            key: key.to_string(),
        }
    }

    pub fn log(&self, level: Level, msg: &str, file: &str, line: u32, column: u32) {
        log::log!(
            level,
            "[{}] [{}:{}:{}] {}",
            self.key,
            file,
            line,
            column,
            msg
        );
    }

    pub fn trace(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Trace, msg, file, line, column);
    }

    pub fn debug(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Debug, msg, file, line, column);
    }

    pub fn info(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Info, msg, file, line, column);
    }

    pub fn warn(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Warn, msg, file, line, column);
    }

    pub fn error(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Error, msg, file, line, column);
    }
}

macro_rules! log_trace {
    ($logger:expr, $msg:expr) => {
        $logger.trace($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.trace(&format!($msg, $($arg)+), file!(), line!(), column!())
    };
}

#[allow(dead_code)]
macro_rules! log_debug {
    ($logger:expr, $msg:expr) => {
        $logger.debug($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.debug(&format!($msg, $($arg)+), file!(), line!(), column!())
    };
}

#[allow(dead_code)]
macro_rules! log_info {
    ($logger:expr, $msg:expr) => {
        $logger.info($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.info(&format!($msg, $($arg)+), file!(), line!(), column!())
    };
}

#[allow(dead_code)]
macro_rules! log_warn {
    ($logger:expr, $msg:expr) => {
        $logger.warn($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.warn(&format!($msg, $($arg)+), file!(), line!(), column!())
    };
}

#[allow(dead_code)]
macro_rules! log_error {
    ($logger:expr, $msg:expr) => {
        $logger.error($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.error(&format!($msg, $($arg)+), file!(), line!(), column!())
    };
}
