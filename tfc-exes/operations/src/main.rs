#[cfg(feature = "opcua-expose")]
use opcua::server::{
    node_manager::memory::{simple_node_manager, NamespaceMetadata, SimpleNodeManager},
    ServerBuilder,
};
use tfc::logger;
use tfc::progbase;
use tokio;

mod operations;

#[cfg(feature = "opcua-expose")]
use crate::operations::server::OpcuaExpose;
use crate::operations::server::OperationsImpl;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    progbase::init();
    logger::init_combined_logger()?;
    let formatted_name = format!(
        "is.centroid.{}.{}",
        progbase::exe_name(),
        progbase::proc_name()
    );
    let bus = zbus::connection::Builder::system()?
        .name(formatted_name)?
        .build()
        .await?;

    let (sm, _handle) = OperationsImpl::spawn(bus.clone());

    #[cfg(feature = "opcua-expose")]
    let (server, handle) = ServerBuilder::new()
        .with_config_from(progbase::make_config_file_name("server", "conf")) // todo as command line arg ?
        .with_node_manager(simple_node_manager(
            NamespaceMetadata {
                namespace_uri: "urn:Operations".into(),
                ..Default::default()
            },
            "Operations",
        ))
        .build()
        .expect("Failed to create OPC UA Server");
    #[cfg(feature = "opcua-expose")]
    sm.lock().await.opcua_expose(
        handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .expect("Failed to get SimpleNodeManager"),
        handle.subscriptions().clone(),
        handle
            .get_namespace_index("urn:Operations")
            .expect("Failed to get namespace index"),
    );
    #[cfg(feature = "opcua-expose")]
    tokio::spawn(async move { server.run().await });

    std::future::pending::<()>().await;
    Ok(())
}
