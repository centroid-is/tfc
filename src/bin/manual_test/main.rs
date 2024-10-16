use tfc::confman::ConfMan;
use tfc::ipc::{dbus, Base, Signal, Slot};
use tfc::logger;
use tfc::progbase;

use log::trace;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::future::pending;
use std::path::PathBuf;
use zbus::connection;

use opcua::server::address_space::Variable;
use opcua::server::address_space::VariableBuilder;
use opcua::server::node_manager::memory::{
    simple_node_manager, InMemoryNodeManager, NamespaceMetadata, SimpleNodeManager,
    SimpleNodeManagerImpl,
};
use opcua::server::{ServerBuilder, SubscriptionCache};
use opcua::types::{DataValue, NodeId, UAString};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

    // Create an OPC UA server with sample configuration and default node set
    let (server, handle) = ServerBuilder::new()
        .with_config_from("src/bin/manual_test/server.conf")
        .with_node_manager(simple_node_manager(
            NamespaceMetadata {
                namespace_uri: "urn:SimpleServer".to_owned(),
                ..Default::default()
            },
            "simple",
        ))
        .build()
        .unwrap();
    let node_manager = handle
        .node_managers()
        .get_of_type::<SimpleNodeManager>()
        .unwrap();
    let ns = handle.get_namespace_index("urn:SimpleServer").unwrap();

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

    let bool_slot = Slot::<bool>::new(_conn.clone(), Base::new("bar", None));
    let mut i64_slot = Slot::<i64>::new(_conn.clone(), Base::new("bar", None));
    let mut i64_slot_stream = Slot::<i64>::new(_conn.clone(), Base::new("stream", None));
    i64_slot.recv(Box::new(|&val| {
        println!("Received value: {:?}", val);
    }));
    println!("Slot created");
    dbus::SignalInterface::register(i64_signal.base(), _conn.clone(), i64_signal.subscribe());
    dbus::SlotInterface::register(i64_slot.base(), _conn.clone(), i64_slot.channel("dbus"));
    dbus::SlotInterface::register(
        i64_slot_stream.base(),
        _conn.clone(),
        i64_slot_stream.channel("dbus"),
    );

    let signal_node = NodeId::new(ns, i64_signal.base().name);
    let slot_node = NodeId::new(ns, i64_slot.base().name);
    let address_space = node_manager.address_space();
    let tfc_folder_id = NodeId::new(ns, "folder");
    {
        let mut address_space = address_space.write();

        address_space.add_folder(&tfc_folder_id, "TFC", "TFC", &NodeId::objects_folder_id());

        // VariableBuilder::new(&slot_node, i64_slot.base().name, i64_slot.base().name)
        //     .data_type(opcua::types::DataTypeId::Int64)
        //     .writable()
        //     .organized_by(&tfc_folder_id)
        //     .insert(&mut address_space);

        // let _ = address_space.add_variables(
        //     vec![Variable::new(
        //         &signal_node,
        //         i64_signal.base().name,
        //         i64_signal.base().name,
        //         0_i64,
        //     )],
        //     &tfc_folder_id,
        // );
    }

    tfc::ipc::opcua::SignalInterface::register_node(
        i64_signal.base(),
        i64_signal.subscribe(),
        signal_node,
        tfc_folder_id.clone(),
        node_manager.clone(),
        handle.subscriptions().clone(),
    );
    tfc::ipc::opcua::SlotInterface::register_node(
        i64_slot.base(),
        i64_slot.channel("opcua"),
        slot_node,
        tfc_folder_id,
        node_manager.clone(),
    );

    let full_name = i64_signal.base().full_name();
    let _ = i64_slot.async_connect(&full_name).await;
    let _ = i64_slot_stream.async_connect(&full_name).await;
    println!("Slot connected");

    let mut stream = i64_slot_stream.subscribe();
    tokio::spawn(async move {
        loop {
            println!("awaiting new stream val");
            let val = stream.changed().await;
            println!(
                "Watch val: {:?}, val: {:?}",
                val,
                stream.borrow_and_update()
            );
        }
    });

    tokio::spawn(async move {
        loop {
            println!("awaiting new bool val");
            let val = bool_slot.async_recv().await;
            println!("Bool slot val: {:?}", val);
        }
    });

    tokio::spawn(async move {
        for i in 1..1024 {
            i64_signal.async_send(i).await.expect("Failed to send");
            println!("Sending value: {}", i);
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });

    // add_example_variables(ns, node_manager, handle.subscriptions().clone());

    tokio::spawn(async move {
        server.run().await.expect("Failed to run server");
    });

    // Do other things or go to wait forever
    pending::<()>().await;

    Ok(())
}

fn add_example_variables(
    ns: u16,
    manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    subscriptions: Arc<SubscriptionCache>,
) {
    // These will be the node ids of the new variables
    let v1_node = NodeId::new(ns, "v1");
    let v2_node = NodeId::new(ns, "v2");
    let v3_node = NodeId::new(ns, "v3");
    let v4_node = NodeId::new(ns, "v4");
    let v5_node = NodeId::new(ns, "v5");

    let address_space = manager.address_space();

    // The address space is guarded so obtain a lock to change it
    {
        let mut address_space = address_space.write();

        // Create a sample folder under objects folder
        let sample_folder_id = NodeId::new(ns, "folder");
        address_space.add_folder(
            &sample_folder_id,
            "Sample",
            "Sample",
            &NodeId::objects_folder_id(),
        );

        // Add some variables to our sample folder. Values will be overwritten by the timer
        let _ = address_space.add_variables(
            vec![
                Variable::new(&v1_node, "v1", "v1", 0_i32),
                Variable::new(&v2_node, "v2", "v2", false),
                Variable::new(&v3_node, "v3", "v3", UAString::from("")),
                Variable::new(&v4_node, "v4", "v4", 0f64),
                Variable::new(&v5_node, "v5", "v5", "Static Value"),
            ],
            &sample_folder_id,
        );
    }

    // Depending on your choice of node manager, you can use different methods to provide the value of a node.
    // The simple node manager lets you set dynamic getters:
    {
        let counter = AtomicI32::new(0);
        manager
            .inner()
            .add_read_callback(v3_node.clone(), move |_, _, _| {
                Ok(DataValue::new_now(UAString::from(format!(
                    "Hello World times {}",
                    counter.fetch_add(1, Ordering::Relaxed)
                ))))
            });

        let start_time = Instant::now();
        manager
            .inner()
            .add_read_callback(v4_node.clone(), move |_, _, _| {
                let elapsed = (Instant::now() - start_time).as_millis();
                let moment = (elapsed % 10_000) as f64 / 10_000.0;
                Ok(DataValue::new_now(
                    (2.0 * std::f64::consts::PI * moment).sin(),
                ))
            });
    }

    // Alternatively, you can set the value in the node manager on a timer.
    // This is typically a better choice if updates are relatively rare, and you always know when
    // an update occurs. Fundamentally, the server is event-driven. When using a getter like above,
    // the node manager will sample the value if a user subscribes to it. When publishing a value like below,
    // clients will only be notified when a change actually happens, but we will need to store each new value.

    // Typically, you will use a getter or a custom node manager for dynamic values, and direct modification for
    // properties or other less-commonly changing values.
    {
        // Store a counter and a flag in a tuple
        let counter = AtomicI32::new(0);
        let flag = AtomicBool::new(false);
        tokio::task::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(300));
            loop {
                interval.tick().await;

                manager
                    .set_values(
                        &subscriptions,
                        [
                            (
                                &v1_node,
                                None,
                                DataValue::new_now(counter.fetch_add(1, Ordering::Relaxed)),
                            ),
                            (
                                &v2_node,
                                None,
                                DataValue::new_now(flag.fetch_xor(true, Ordering::Relaxed)),
                            ),
                        ]
                        .into_iter(),
                    )
                    .unwrap();
            }
        });
    }
}
