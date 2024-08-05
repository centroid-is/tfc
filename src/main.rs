mod progbase;

use log::info;


fn main() {
    progbase::init();


    info!("Program started with ID: {}", progbase::proc_name());
    println!("Program started with ID: {}", progbase::proc_name());
    println!("Config directory: {}", progbase::config_directory().display());
}