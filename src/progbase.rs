include!(concat!(env!("OUT_DIR"), "/version.rs"));

use clap::Parser;
use log::LevelFilter;
use std::env;
use std::path::PathBuf;
use std::process::exit;
use std::sync::{Arc, Mutex};
#[derive(Parser, Default)]
pub struct Opts {
    #[clap(short, long, default_value = "def")]
    pub id: String,
    #[clap(short, long, default_value = "info")]
    pub log_level: String,
    #[clap(short, long)]
    pub version: bool,
    #[clap(short, long)]
    pub stdout: bool,
}

pub struct Options {
    pub exe: String,
    pub id: String,
    pub log_level: LevelFilter,
    pub stdout: bool,
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
            stdout: opts.stdout,
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
        stdout: false,
    }));
}

pub fn try_init() -> Result<(), Box<dyn std::error::Error>> {
    let mut options = OPTIONS.lock().unwrap();
    let mut opts = Opts::default();
    opts.id = "def".to_string(); // todo would be nice to use the default from clap

    let res = opts.try_update_from(std::env::args());
    if opts.version {
        /*
        git@github.com:centroid-is/framework-rs.git
        Build date: 2024-08-05 18:26:11
        Commit date: 2024-08-05
        Branch: main
        Hash: 43aa4af47ee2c6aa269f10c9ff9e1cd3b1c1e259
        Tag:  - clean
        Author: Jón Bjarni Bjarnason <jon@centroid.is>
        */
        println!("{}", GIT_REPO);
        println!("Build date: {}", BUILD_DATE);
        println!("Commit date: {}", GIT_COMMIT_DATE);
        println!("Branch: {}", GIT_BRANCH);
        println!("Hash: {}", GIT_HASH);
        println!("Tag: {} - {}", GIT_TAG, GIT_IS_DIRTY);
        println!("Author: {}", GIT_AUTHOR);
        exit(0)
    }
    *options = Options::new(opts);
    Ok(res?)
}

pub fn init() {
    let res = try_init();
    if res.is_err() {
        panic!("Failed to initialize program base: {}", res.err().unwrap());
    }
}

#[allow(dead_code)]
pub fn exe_name() -> String {
    // todo how to return std::string const&
    let options = OPTIONS.lock().unwrap();
    options.exe.clone()
}

#[allow(dead_code)]
pub fn proc_name() -> String {
    // todo samesies
    let options = OPTIONS.lock().unwrap();
    options.id.clone()
}

#[allow(dead_code)]
pub fn log_lvl() -> LevelFilter {
    let options = OPTIONS.lock().unwrap();
    options.log_level
}

#[allow(dead_code)]
pub fn stdout() -> bool {
    let options = OPTIONS.lock().unwrap();
    options.stdout
}

#[allow(dead_code)]
pub fn config_directory() -> PathBuf {
    env::var("CONFIGURATION_DIRECTORY").map_or_else(|_| PathBuf::from("/etc/tfc/"), PathBuf::from)
}

#[allow(dead_code)]
pub fn make_config_file_name(filename: &str, extension: &str) -> PathBuf {
    let config_dir = config_directory();
    config_dir
        .join(exe_name())
        .join(proc_name())
        .join(format!("{}.{}", filename, extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progbase_test() {
        println!("Program started with ID: {}", proc_name());
        println!("Config directory: {}", config_directory().display());
    }

    #[test]
    fn init_test() {
        let _ = try_init();
        let exe_name = exe_name();
        println!("exe_name: {}", exe_name);
        println!("proc_name: {}", proc_name());
        assert_eq!(
            exe_name,
            std::env::current_exe()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        );
    }
}
