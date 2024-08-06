use log::{self, Level};
use std::sync::Once;
use std::fmt::Debug;

static INIT: Once = Once::new();

pub struct Logger {
    key: String,
}

impl Logger {
    pub fn new(key: &str) -> Self {
        INIT.call_once(|| {
            let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init();
        });

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

    pub fn log_fmt(&self, level: Level, msg: &str, args: impl Debug, file: &str, line: u32, column: u32) {
        let formatted_msg = format!("{} - {:?}", msg, args);
        self.log(level, &formatted_msg, file, line, column);
    }

    pub fn trace(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Trace, msg, file, line, column);
    }

    pub fn trace_fmt(&self, msg: &str, args: impl Debug, file: &str, line: u32, column: u32) {
        self.log_fmt(Level::Trace, msg, args, file, line, column);
    }

    pub fn debug(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Debug, msg, file, line, column);
    }

    pub fn debug_fmt(&self, msg: &str, args: impl Debug, file: &str, line: u32, column: u32) {
        self.log_fmt(Level::Debug, msg, args, file, line, column);
    }

    pub fn info(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Info, msg, file, line, column);
    }

    pub fn info_fmt(&self, msg: &str, args: impl Debug, file: &str, line: u32, column: u32) {
        self.log_fmt(Level::Info, msg, args, file, line, column);
    }

    pub fn warn(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Warn, msg, file, line, column);
    }

    pub fn warn_fmt(&self, msg: &str, args: impl Debug, file: &str, line: u32, column: u32) {
        self.log_fmt(Level::Warn, msg, args, file, line, column);
    }

    pub fn error(&self, msg: &str, file: &str, line: u32, column: u32) {
        self.log(Level::Error, msg, file, line, column);
    }

    pub fn error_fmt(&self, msg: &str, args: impl Debug, file: &str, line: u32, column: u32) {
        self.log_fmt(Level::Error, msg, args, file, line, column);
    }
}

#[allow(dead_code)]
macro_rules! log_trace {
    ($logger:expr, $msg:expr) => {
        $logger.trace($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.trace_fmt($msg, ($($arg)+), file!(), line!(), column!())
    };
}

#[allow(dead_code)]
macro_rules! log_debug {
    ($logger:expr, $msg:expr) => {
        $logger.debug($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.debug_fmt($msg, ($($arg)+), file!(), line!(), column!())
    };
}

#[allow(dead_code)]
macro_rules! log_info {
    ($logger:expr, $msg:expr) => {
        $logger.info($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.info_fmt($msg, ($($arg)+), file!(), line!(), column!())
    };
}

#[allow(dead_code)]
macro_rules! log_warn {
    ($logger:expr, $msg:expr) => {
        $logger.warn($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.warn_fmt($msg, ($($arg)+), file!(), line!(), column!())
    };
}

#[allow(dead_code)]
macro_rules! log_error {
    ($logger:expr, $msg:expr) => {
        $logger.error($msg, file!(), line!(), column!())
    };
    ($logger:expr, $msg:expr, $($arg:tt)+) => {
        $logger.error_fmt($msg, ($($arg)+), file!(), line!(), column!())
    };
}