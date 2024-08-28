mod confman;
mod ipc;
mod logger;
mod progbase;

use std::future::pending;

use confman::ConfMan;
use ipc::{Base, Signal, Slot};
use log::{log, Level};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use zbus::{connection, interface};

#[derive(Deserialize, Serialize, JsonSchema, Default)]
struct Greeter {
    count: u64,
}

// #[interface(name = "org.zbus.MyGreeter1")]
// impl Greeter {
//     // Can be `async` as well.
//     fn say_hello(&mut self, name: &str) -> String {
//         self.count += 1;
//         format!("Hello {}! I have been called {} times.", name, self.count)
//     }
// }

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Begin");
    progbase::init();
    let _ = logger::init_combined_logger();

    let _conn = connection::Builder::session()?
        .name(format!(
            "is.centroid.{}.{}",
            progbase::exe_name(),
            progbase::proc_name()
        ))?
        .build()
        .await?;

    println!("End");

    let _config = ConfMan::<Greeter>::new(_conn.clone(), "greeterfu_uf0");

    let mut i64_signal = Signal::<i64>::new(Base::new("foo", None));
    i64_signal.init().await?;

    let mut i64_slot = Slot::<i64>::new(Base::new("bar", None));
    i64_slot.connect(i64_signal.full_name().as_str()).await?;
    tokio::spawn(async move {
        println!("Wait for recv");
        let val = i64_slot.recv().await;
        println!("Received value: {:?}", val);
    });

    for i in 1..1024 {
        i64_signal.send(i).await?;
        println!("The value of i is: {}", i);
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    // Do other things or go to wait forever
    pending::<()>().await;

    Ok(())
}
