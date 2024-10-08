use tfc::confman::ConfMan;
use tfc::ipc::{Base, Signal, Slot, SlotImpl};
use tfc::logger;
use tfc::progbase;

use futures::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::future::pending;
use zbus::connection;

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
//
// struct Application {
//     my_dream: i64,
//     slot: Slot<i64>,
// }

// impl Application {
//     fn new(signal_name: &str) -> Arc<Mutex<Self>> {
//         let app = Arc::new(Mutex::new(Application {
//             my_dream: 0,
//             slot: Slot::new(Base::new("bark", None)),
//         }));

//         let shared_app = Arc::clone(&app);
//         app.lock().unwrap().slot.recv(Box::new(move |val: &i64| {
//             shared_app.lock().unwrap().callback(val);
//         }));

//         let _ = app.lock().unwrap().slot.connect(signal_name);

//         app
//     }
//     fn callback(&mut self, val: &i64) {
//         self.my_dream = *val;
//         println!("All my dreams come true: {}", val);
//     }
// }

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Begin");
    progbase::init();
    let _ = logger::init_combined_logger();

    let _conn = connection::Builder::system()?
        .name(format!(
            "is.centroid.{}.{}",
            progbase::exe_name(),
            progbase::proc_name()
        ))?
        .build()
        .await?;

    println!("End");

    let _config = ConfMan::<Greeter>::new(_conn.clone(), "greeterfu_uf0");

    let mut i64_signal = Signal::<i64>::new(_conn.clone(), Base::new("foo", None));

    // let _ = Application::new(&i64_signal.full_name());

    let mut i64_raw_slot = SlotImpl::<i64>::new(Base::new("hello", None));
    let mut bool_slot = Slot::<bool>::new(_conn.clone(), Base::new("bar", None));
    let mut i64_slot = Slot::<i64>::new(_conn.clone(), Base::new("bar", None));
    let mut i64_slot_stream = Slot::<i64>::new(_conn.clone(), Base::new("stream", None));
    i64_slot.recv(Box::new(|&val| {
        println!("Received value: {:?}", val);
    }));
    println!("Slot created");
    let _ = i64_raw_slot.connect(i64_signal.full_name());
    let _ = i64_slot.connect(i64_signal.full_name());
    let _ = i64_slot_stream.connect(i64_signal.full_name());
    println!("Slot connected");

    let mut stream = i64_slot_stream.stream();
    tokio::spawn(async move {
        loop {
            println!("awaiting new stream val");
            let val = stream.next().await;
            println!("Stream val: {:?}", val);
        }
    });

    tokio::spawn(async move {
        loop {
            println!("awaiting new val");
            let val = i64_raw_slot.recv().await;
            println!("Raw slot val: {:?}", val);
        }
    });

    tokio::spawn(async move {
        loop {
            println!("awaiting new bool val");
            let val = bool_slot.async_recv().await;
            println!("Bool slot val: {:?}", val);
        }
    });

    for i in 1..1024 {
        i64_signal
            .async_send(i)
            .await
            .expect("This should not fail");
        println!("The value of i is: {}", i);
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    // Do other things or go to wait forever
    pending::<()>().await;

    Ok(())
}
