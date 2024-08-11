use log::{self, Level, Log, Metadata, Record, SetLoggerError};
use std::sync::Once;
use systemd_journal_logger::JournalLog;

use crate::progbase;

static INIT: Once = Once::new();

struct CombinedLogger {
    env_logger: env_logger::Logger,
    journal_logger: JournalLog,
}

impl Log for CombinedLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.env_logger.enabled(metadata) || self.journal_logger.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if self.env_logger.enabled(record.metadata()) {
            self.env_logger.log(record);
        }
        if self.journal_logger.enabled(record.metadata()) {
            self.journal_logger.log(record);
        }
    }

    fn flush(&self) {
        self.env_logger.flush();
        self.journal_logger.flush();
    }
}

fn init_combined_logger() -> Result<(), SetLoggerError> {
    INIT.call_once(|| {
        let env_logger: env_logger::Logger =
            env_logger::Builder::from_env(env_logger::Env::default())
                .filter_level(if progbase::stdout() { progbase::log_lvl() } else { log::LevelFilter::Off })
                .build();
            
        let journal_logger = JournalLog::new().unwrap().with_extra_fields(vec![("TFC_EXE", progbase::exe_name()), ("TFC_ID", progbase::proc_name())]);
        log::set_max_level(progbase::log_lvl());

        let combined_logger = CombinedLogger {
            env_logger,
            journal_logger,
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
