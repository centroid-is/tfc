mod progbase;
#[macro_use]
mod logger;

use log::{info, log, Level};

fn main() {
    println!("Begin");
    progbase::init();
    let _ = logger::init_combined_logger();

    log!(target: "TFC_KEY1", Level::Trace, "This is a trace message");
    log!(target: "TFC_KEY2", Level::Debug, "This is a debug message");
    log!(target: "TFC_KEY3", Level::Info, "This is a info message");
    log!(target: "TFC_KEY4", Level::Warn, "This is a warn message");
    log!(target: "TFC_KEY5", Level::Error, "This is a error message");

    log!(target: "TFC_KEY1", Level::Trace, "This is a {} trace message with {} args", 42, "foo");
    log!(target: "TFC_KEY1", Level::Debug, "This is a debug {} {} message with args", 42, "foo");
    log!(target: "TFC_KEY1", Level::Info, "{} This is an {} info message with args", 42, "foo");
    log!(target: "TFC_KEY1", Level::Warn, "This is a {} warn message {} with args", 42, "foo");
    log!(target: "TFC_KEY1", Level::Error, "This is an {} error message with {} args", 42, "foo");

    info!("Program started with ID: {}", progbase::proc_name());
    println!("Program started with ID: {}", progbase::proc_name());
    println!(
        "Config directory: {}",
        progbase::config_directory().display()
    );
    println!("End");
}
