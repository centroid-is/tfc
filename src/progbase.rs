use clap::Parser;
use std::env;
use std::path::PathBuf;
use log::LevelFilter;
use std::sync::{Arc, Mutex};

#[derive(Parser)]
pub struct Opts {
    #[clap(short, long, default_value = "def")]
    pub id: String,
    #[clap(short, long, default_value = "info")]
    pub log_level: String,
    #[clap(short, long)]
    pub version: bool,
}

pub struct Options {
    pub exe: String,
    pub id: String,
    pub log_level: LevelFilter,
}

impl Options {
    pub fn new(opts: Opts) -> Self {
        let exe_name = std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let log_level = match opts.log_level.as_str() {
            "error" => LevelFilter::Error,
            "warn" => LevelFilter::Warn,
            "info" => LevelFilter::Info,
            "debug" => LevelFilter::Debug,
            "trace" => LevelFilter::Trace,
            _ => LevelFilter::Info,
        };

        Options {
            exe: exe_name,
            id: opts.id,
            log_level: log_level,
        }
    }
}

// Singleton for Options
lazy_static::lazy_static! {
    // https://users.rust-lang.org/t/how-can-i-use-mutable-lazy-static/3751/6
    static ref OPTIONS: Arc<Mutex<Options>> = Arc::new(Mutex::new(Options {
        exe: String::new(),
        id: String::new(),
        log_level: LevelFilter::Info,
    }));
}

pub fn init() {
    let mut options = OPTIONS.lock().unwrap();
    let opts = Opts::parse();
    *options = Options::new(opts);
}

#[allow(dead_code)]
pub fn exe_name() -> String { // todo how to return std::string const&
    let options = OPTIONS.lock().unwrap();
    options.exe.clone()
}

#[allow(dead_code)]
pub fn proc_name() -> String { // todo samesies
    let options = OPTIONS.lock().unwrap();
    options.id.clone()
}

#[allow(dead_code)]
pub fn log_lvl() -> LevelFilter {
    let options = OPTIONS.lock().unwrap();
    options.log_level
}

#[allow(dead_code)]
pub fn config_directory() -> PathBuf {
    env::var("CONFIGURATION_DIRECTORY").map_or_else(|_| PathBuf::from("/etc/tfc/"), PathBuf::from)
}

#[allow(dead_code)]
pub fn make_config_file_name(filename: &str, extension: &str) -> PathBuf {
    let config_dir = config_directory();
    config_dir.join(exe_name()).join(proc_name()).join(format!("{}.{}", filename, extension))
}


