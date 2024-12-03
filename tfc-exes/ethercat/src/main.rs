use log::debug;
use tfc::logger;
use tfc::progbase;

mod bus;
mod devices;
#[cfg(feature = "opcua-expose")]
mod opcua;

use crate::bus::Bus;
#[cfg(feature = "opcua-expose")]
use crate::opcua::OpcuaServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // console_subscriber::init();
    progbase::init();
    println!("Starting ethercat");
    println!("{}", progbase::exe_name());
    println!("{}", progbase::proc_name());
    logger::init_combined_logger()?;
    debug!(target: "ethercat", "Starting ethercat");

    let formatted_name = format!(
        "is.centroid.{}.{}",
        progbase::exe_name(),
        progbase::proc_name()
    );
    let dbus = zbus::connection::Builder::system()?
        .name(formatted_name)?
        .build()
        .await?;

    #[cfg(feature = "opcua-expose")]
    let opcua_server = OpcuaServer::new(
        "/etc/tfc/ethercat/def/server.conf", // todo as command line arg ?
        "urn:Ethercat",
        "Ethercat",
    );
    #[cfg(feature = "opcua-expose")]
    let opcua_handle = opcua_server.make_handle(
        opcua_server
            .handle
            .get_namespace_index("urn:Ethercat")
            .expect("Failed to get namespace index"),
    );

    let mut bus = Bus::new(
        dbus.clone(),
        #[cfg(feature = "opcua-expose")]
        opcua_handle,
    );

    tokio::spawn(async move { bus.init_and_run(dbus).await });

    #[cfg(feature = "opcua-expose")]
    tokio::spawn(async move { opcua_server.server.run().await });

    std::future::pending::<()>().await;
    Ok(())
}
