use log::{self, Log, Metadata, Record, SetLoggerError};
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
            // Example output from journalctl
            /*
            PRIORITY=3
            CODE_FILE=src/main.rs
            CODE_MODULE=framework_rs
            TFC_EXE=framework-rs
            TFC_ID=def
            TARGET=TFC_KEY1
            MESSAGE=This is an 42 error message with foo args
            CODE_LINE=22
            SYSLOG_IDENTIFIER=framework-rs.def
            SYSLOG_PID=41176
             */
            self.journal_logger.log(record);
        }
    }

    fn flush(&self) {
        self.env_logger.flush();
        self.journal_logger.flush();
    }
}

pub fn init_combined_logger() -> Result<(), SetLoggerError> {
    INIT.call_once(|| {
        let env_logger: env_logger::Logger =
            env_logger::Builder::from_env(env_logger::Env::default())
                .filter_level(if progbase::stdout() { progbase::log_lvl() } else { log::LevelFilter::Off })
                .build();
            
        let journal_logger = JournalLog::new().unwrap()
            .with_extra_fields(vec![("TFC_EXE", progbase::exe_name()), ("TFC_ID", progbase::proc_name())])
            .with_syslog_identifier(format!("{}.{}", progbase::exe_name(), progbase::proc_name()));
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
