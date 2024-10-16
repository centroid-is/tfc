use crate::ipc::Base;
use log::{error, trace, warn};
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
    opcua::types::Variant: OpcuaSupportedTypes<T>,
    opcua::types::Variant: From<T>,
{
    pub fn register_node(
        base: Base<T>,
        channel: (watch::Sender<Option<T>>, watch::Receiver<Option<T>>),
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
        manager
            .inner()
            .add_write_callback(node.clone(), move |value, _| {
                error!("slot write callback");
                if value.value.is_none() {
                    return opcua::types::StatusCode::BadNoValue;
                }

                let val: Result<T, Box<dyn std::error::Error + 'static>> =
                    value.value.unwrap().from_variant();
                if let Ok(v) = val {
                    channel.0.send(Some(v)).expect("Failed to send value");
                } else {
                    warn!("Unable to convert value: {:?}", val.err());
                }

                opcua::types::StatusCode::Good
            });
        let mut watch = channel.1;
        tokio::spawn(async move {
            loop {
                watch.changed().await.expect("Failed to wait for change");
                let val = *watch.borrow_and_update();
                match val {
                    Some(val) => {
                        trace!(target: &base.log_key, "opcua slot is {:?}", val);

                        manager
                            .set_value(&subscriptions, &node, None, DataValue::new_now(val))
                            .expect("Failed to set value");
                    }
                    None => {
                        trace!(target: &base.log_key, "slot is None");
                    }
                }
            }
        });
    }
}

pub trait OpcuaSupportedTypes<T> {
    fn from_variant(&self) -> Result<T, Box<dyn std::error::Error>>
    where
        Self: Sized;
}

macro_rules! impl_opcua_supported_types {
    ($(($rust_type:ty, $variant_type:ident)),*) => {
        $(
            impl OpcuaSupportedTypes<$rust_type> for opcua::types::Variant {
                fn from_variant(&self) -> Result<$rust_type, Box<dyn std::error::Error>> {
                    match self {
                        opcua::types::Variant::$variant_type(value) => Ok(*value),
                        _ => Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!("Invalid variant type for {}: {:?}", stringify!($rust_type), self),
                        ))),
                    }
                }
            }
        )*
    };
}

impl_opcua_supported_types!(
    (i64, Int64),
    (i32, Int32),
    (i16, Int16),
    (i8, SByte),
    (u64, UInt64),
    (u32, UInt32),
    (u16, UInt16),
    (u8, Byte),
    (f64, Double),
    (f32, Float),
    (bool, Boolean) //,
                    // (String, String)
);
