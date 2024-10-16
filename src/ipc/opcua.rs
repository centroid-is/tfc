use crate::ipc::Base;
use log::{error, trace};
use opcua::{
    server::{
        address_space::VariableBuilder,
        node_manager::memory::{InMemoryNodeManager, SimpleNodeManagerImpl},
        SubscriptionCache,
    },
    types::{DataValue, NodeId, Variant},
};
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::watch;
pub struct SignalInterface<T> {
    value_receiver: watch::Receiver<Option<T>>,
    log_key: String,
    watch_task: tokio::task::JoinHandle<()>,
}

impl<T: Clone + Copy + Sync + Send + 'static + std::fmt::Debug> SignalInterface<T>
where
    opcua::types::Variant: From<T>,
{
    pub fn register_node(
        base: Base<T>,
        mut watch: watch::Receiver<Option<T>>,
        node: NodeId,
        parent_node: NodeId,
        manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
        subscriptions: Arc<SubscriptionCache>,
    ) {
        let address_space = manager.address_space();
        {
            let mut address_space = address_space.write();
            VariableBuilder::new(&node, &base.name, &base.name)
                .data_type(opcua::types::DataTypeId::Int64)
                .writable()
                .organized_by(&parent_node)
                .insert(&mut address_space);
        }
        tokio::spawn(async move {
            loop {
                watch.changed().await.expect("Failed to wait for change");
                let val = *watch.borrow_and_update();
                match val {
                    Some(val) => {
                        trace!(target: &base.log_key, "opcua signal is {:?}", val);

                        manager
                            .set_value(&subscriptions, &node, None, DataValue::new_now(val))
                            .expect("Failed to set value");
                    }
                    None => {
                        trace!(target: &base.log_key, "signal is None");
                    }
                }
            }
        });
    }
}

pub struct SlotInterface<T> {
    _marker: PhantomData<T>,
}

impl<T: Clone + Copy + Sync + Send + 'static + std::fmt::Debug> SlotInterface<T>
where
    opcua::types::Variant: From<T>,
{
    pub fn register_node(
        base: Base<T>,
        channel: (watch::Sender<Option<T>>, watch::Receiver<Option<T>>),
        node: NodeId,
        parent_node: NodeId,
        manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    ) {
        let address_space = manager.address_space();
        {
            let mut address_space = address_space.write();
            VariableBuilder::new(&node, &base.name, &base.name)
                .data_type(opcua::types::DataTypeId::Int64)
                .writable()
                .organized_by(&parent_node)
                .insert(&mut address_space);
        }
        manager
            .inner()
            .add_write_callback(node.clone(), move |value, _| {
                error!("slot write callback");
                // if let Some(Variant::Int64(v)) = value {
                //     channel.0.send(Some(v));
                // }

                opcua::types::StatusCode::Good
            });
    }
}
