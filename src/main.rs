mod progbase;
#[macro_use]
mod logger;
use logger::Logger;

use log::info;


fn main() {
    progbase::init();

    let logger = Logger::new("example-component");

    log_trace!(logger, "This is a trace message");
    log_debug!(logger, "This is a debug message");
    log_info!(logger, "This is an info message");
    log_warn!(logger, "This is a warn message");
    log_error!(logger, "This is an error message");

    log_trace!(logger, "This is a trace message with args", 42, "foo");
    log_debug!(logger, "This is a debug message with args", 42, "foo");
    log_info!(logger, "This is an info message with args", 42, "foo");
    log_warn!(logger, "This is a warn message with args", 42, "foo");
    log_error!(logger, "This is an error message with args", 42, "foo");

    info!("Program started with ID: {}", progbase::proc_name());
    println!("Program started with ID: {}", progbase::proc_name());
    println!("Config directory: {}", progbase::config_directory().display());
}