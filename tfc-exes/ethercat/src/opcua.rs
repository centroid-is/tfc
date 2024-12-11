use opcua::server::{
    node_manager::memory::{
        simple_node_manager, InMemoryNodeManager, NamespaceMetadata, SimpleNodeManager,
        SimpleNodeManagerImpl,
    },
    ServerBuilder, SubscriptionCache,
};
use std::path::PathBuf;
use std::sync::Arc;

pub struct OpcuaServer {
    pub server: opcua::server::Server,
    pub handle: opcua::server::ServerHandle,
}

impl OpcuaServer {
    pub fn new(
        path: impl Into<PathBuf>,
        namespace_uri: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        let (server, handle) = ServerBuilder::new()
            .with_config_from(path)
            .with_node_manager(simple_node_manager(
                NamespaceMetadata {
                    namespace_uri: namespace_uri.into(),
                    ..Default::default()
                },
                name.into().as_str(),
            ))
            .build()
            .expect("Failed to create OPC UA Server");
        Self { server, handle }
    }
    pub fn make_handle(&self, namespace: u16) -> OpcuaServerHandle {
        OpcuaServerHandle {
            manager: self
                .handle
                .node_managers()
                .get_of_type::<SimpleNodeManager>()
                .expect("Failed to get SimpleNodeManager"),
            subscriptions: self.handle.subscriptions().clone(),
            namespace,
        }
    }
}
pub struct OpcuaServerHandle {
    pub manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    pub subscriptions: Arc<SubscriptionCache>,
    pub namespace: u16,
}
