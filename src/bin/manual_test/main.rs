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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Deserialize, Serialize, JsonSchema, Default)]
struct Greeter {
    count: u64,
}

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
    let mut string_signal = Signal::<String>::new(_conn.clone(), Base::new("hello", None));

    string_signal
        .async_send("hello world".to_string())
        .await
        .expect("Failed to send");

    // let _ = Application::new(&i64_signal.full_name());

    let bool_slot = Slot::<bool>::new(_conn.clone(), Base::new("bar", None));
    let mut i64_slot = Slot::<i64>::new(_conn.clone(), Base::new("bar", None));
    let mut i64_slot2 = Slot::<i64>::new(_conn.clone(), Base::new("bar2", None));
    let mut i64_slot_stream = Slot::<i64>::new(_conn.clone(), Base::new("stream", None));
    let mut my_slots = vec![];
    let mut my_signals = vec![];
    let slot_count = 20000;
    for x in 0..(slot_count / 100) {
        my_signals.push(Signal::<i64>::new(
            _conn.clone(),
            Base::new(format!("spawned_pimp{}", x).as_ref(), None),
        ));
    }
    for x in 0..slot_count {
        my_slots.push(Slot::<i64>::new(
            _conn.clone(),
            Base::new(format!("spawned_bitch{}", x).as_ref(), None),
        ));
    }
    let mut i64_slot_stream2 = Slot::<i64>::new(_conn.clone(), Base::new("stream2", None));
    i64_slot.recv(Box::new(|&val| {
        println!("Received value: {:?}", val);
    }));
    i64_slot2.recv(Box::new(|&val| {
        println!("2 Received value: {:?}", val);
    }));
    println!("Slot created");
    // dbus::SignalInterface::register(i64_signal.base(), _conn.clone(), i64_signal.subscribe());
    // dbus::SlotInterface::register(i64_slot.base(), _conn.clone(), i64_slot.channel("dbus"));
    // dbus::SlotInterface::register(
    //     i64_slot_stream.base(),
    //     _conn.clone(),
    //     i64_slot_stream.channel("dbus"),
    // );
    // dbus::SlotInterface::register(
    //     i64_slot_stream2.base(),
    //     _conn.clone(),
    //     i64_slot_stream2.channel("dbus"),
    // );
    // dbus::SlotInterface::register(i64_slot2.base(), _conn.clone(), i64_slot2.channel("dbus"));

    let signal_node = NodeId::new(ns, i64_signal.base().name);
    let slot_node = NodeId::new(ns, i64_slot.base().name);
    let address_space = node_manager.address_space();
    let tfc_folder_id = NodeId::new(ns, "TFC");
    {
        let mut address_space = address_space.write();

        address_space.add_folder(&tfc_folder_id, "TFC", "TFC", &NodeId::objects_folder_id());
    }
    let tfc_folder_id2 = NodeId::new(ns, "TFC2");
    {
        let mut address_space = address_space.write();

        address_space.add_folder(&tfc_folder_id2, "TFC2", "TFC2", &tfc_folder_id);
    }
    let tfc_folder_id3 = NodeId::new(ns, "TFC3");
    {
        let mut address_space = address_space.write();

        address_space.add_folder(&tfc_folder_id3, "TFC3", "TFC3", &tfc_folder_id2);
    }

    let full_name = i64_signal.base().full_name();

    tfc::ipc::opcua::SignalInterface::new(
        i64_signal.base(),
        i64_signal.subscribe(),
        node_manager.clone(),
        handle.subscriptions().clone(),
        ns.clone(),
    )
    .node_id(signal_node)
    .register();
    tfc::ipc::opcua::SignalInterface::new(
        string_signal.base(),
        string_signal.subscribe(),
        node_manager.clone(),
        handle.subscriptions().clone(),
        ns.clone(),
    )
    .register();
    let counter = Arc::new(AtomicUsize::new(0));
    for index in 0..slot_count {
        let slot = &mut my_slots[index];
        let full_name = my_signals[index / 100].base().full_name();
        let _ = slot.async_connect(&full_name).await;
        let slot_name = slot.base().name.clone();
        let counter = counter.clone();
        slot.recv(Box::new(move |&val| {
            counter.fetch_add(1, Ordering::Relaxed);
            //println!("{} Received value: {:?}", slot_name, val);
        }));
        // tfc::ipc::opcua::SlotInterface::new(
        //     slot.base(),
        //     slot.channel("opcua"),
        //     node_manager.clone(),
        //     handle.subscriptions().clone(),
        //     ns.clone(),
        // )
        // .register();
    }
    // my_slots.len();
    tfc::ipc::opcua::SlotInterface::new(
        i64_slot.base(),
        i64_slot.channel("opcua"),
        node_manager.clone(),
        handle.subscriptions().clone(),
        ns.clone(),
    )
    .node_id(slot_node)
    .parent_node(tfc_folder_id3)
    .register();
    tfc::ipc::opcua::SlotInterface::new(
        i64_slot2.base(),
        i64_slot2.channel("opcua"),
        node_manager.clone(),
        handle.subscriptions().clone(),
        ns.clone(),
    )
    .register();
    tfc::ipc::opcua::SlotInterface::new(
        i64_slot_stream.base(),
        i64_slot_stream.channel("opcua"),
        node_manager.clone(),
        handle.subscriptions().clone(),
        ns.clone(),
    )
    .register();
    tfc::ipc::opcua::SlotInterface::new(
        i64_slot_stream2.base(),
        i64_slot_stream2.channel("opcua"),
        node_manager.clone(),
        handle.subscriptions().clone(),
        ns.clone(),
    )
    .register();

    // let _ = i64_slot2.async_connect(&full_name).await;
    // let _ = i64_slot.async_connect(&full_name).await;
    // let _ = i64_slot_stream.async_connect(&full_name).await;
    // let _ = i64_slot_stream2.async_connect(&full_name).await;
    println!("Slot connected");

    let mut stream = i64_slot_stream.subscribe();
    tokio::spawn(async move {
        loop {
            println!("awaiting new stream val");
            let val = stream.changed().await;
            println!(
                "Watch val: {:?}, val: {:?}",
                val,
                *stream.borrow_and_update()
            );
        }
    });

    let mut stream = i64_slot_stream2.subscribe();
    tokio::spawn(async move {
        loop {
            println!("2 awaiting new stream val");
            let val = stream.changed().await;
            println!(
                "Watch val: {:?}, val: {:?}",
                val,
                *stream.borrow_and_update()
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

    // tokio::spawn(async move {
    //     for i in 1..1024 {
    //         println!("Sending value: {}", i);
    //         i64_signal.async_send(i).await.expect("Failed to send");
    //         tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    //         println!(
    //             "Last sent reported by bitches: {}",
    //             counter.load(Ordering::Relaxed)
    //         );
    //         counter.store(0, Ordering::Relaxed);
    //     }
    // });

    tokio::spawn(async move {
        for i in 1..1024 {
            println!("Sending value: {}", i);
            for sig in &mut my_signals {
                sig.async_send(i).await.expect("Failed to send");
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            println!(
                "Last sent reported by bitches: {}",
                counter.load(Ordering::Relaxed)
            );
            counter.store(0, Ordering::Relaxed);
        }
    });

    tokio::spawn(async move {
        server.run().await.expect("Failed to run server");
    });

    // Do other things or go to wait forever
    pending::<()>().await;

    Ok(())
}
