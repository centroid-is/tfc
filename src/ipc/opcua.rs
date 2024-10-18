use crate::ipc::Base;
use crate::ipc::TypeName;
use log::{trace, warn};
use opcua::{
    server::{
        address_space::VariableBuilder,
        node_manager::memory::{InMemoryNodeManager, SimpleNodeManagerImpl},
        SubscriptionCache,
    },
    types::{DataTypeId, DataValue, NodeId},
};
use std::sync::Arc;
use tokio::sync::watch;
pub struct SignalInterface<T> {
    base: Base<T>,
    watch: watch::Receiver<Option<T>>,
    node_id: NodeId,
    parent_node_id: Option<NodeId>,
    name: String,
    manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    subscriptions: Arc<SubscriptionCache>,
    ns: u16,
}

impl<T: Clone + Sync + Send + 'static + std::fmt::Debug + TypeName + ToDataTypeId>
    SignalInterface<T>
where
    opcua::types::Variant: From<T>,
{
    pub fn new(
        base: Base<T>,
        watch: watch::Receiver<Option<T>>,
        manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
        subscriptions: Arc<SubscriptionCache>,
        ns: u16,
    ) -> Self {
        let name = base.name.clone();
        let node_id_name = format!("Signal/{}", Base::<T>::type_and_name(&base.name));
        Self {
            base,
            watch,
            node_id: NodeId::new(ns, node_id_name),
            parent_node_id: None,
            name,
            manager,
            subscriptions,
            ns,
        }
    }
    pub fn node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = node_id;
        self
    }
    pub fn parent_node(mut self, parent_node: NodeId) -> Self {
        self.parent_node_id = Some(parent_node);
        self
    }
    pub fn name(mut self, name: String) -> Self {
        self.name = name;
        self
    }
    pub fn register(mut self) {
        if self.parent_node_id.is_none() {
            let tfc_folder_id = NodeId::new(self.ns, "TFC");
            let signal_folder_id = NodeId::new(self.ns, "Signal");
            let type_folder_id = NodeId::new(self.ns, format!("Signal/{}", T::type_name()));
            {
                let mut address_space = self.manager.address_space().write();
                address_space.add_folder(
                    &tfc_folder_id,
                    "TFC",
                    "TFC",
                    &NodeId::objects_folder_id(),
                );
                address_space.add_folder(&signal_folder_id, "Signal", "Signal", &tfc_folder_id);
                address_space.add_folder(
                    &type_folder_id,
                    T::type_name(),
                    T::type_name(),
                    &signal_folder_id,
                );
            }
            self.parent_node_id = Some(type_folder_id);
        }

        let address_space = self.manager.address_space();
        {
            let mut address_space = address_space.write();
            VariableBuilder::new(&self.node_id, &self.name, &self.name)
                .data_type(T::to_data_type_id())
                .writable()
                .organized_by(self.parent_node_id.as_ref().unwrap())
                .insert(&mut address_space);
        }
        tokio::spawn(async move {
            loop {
                let val = self.watch.borrow_and_update().clone();
                match val {
                    Some(val) => {
                        trace!(target: &self.base.log_key, "opcua signal is {:?}", val);

                        self.manager
                            .set_value(
                                &self.subscriptions,
                                &self.node_id,
                                None,
                                DataValue::new_now(val),
                            )
                            .expect("Failed to set value");
                    }
                    None => {
                        warn!(target: &self.base.log_key, "opc ua got None value from signal");
                    }
                }
                self.watch
                    .changed()
                    .await
                    .expect("Failed to wait for change");
            }
        });
    }
}

pub struct SlotInterface<T> {
    base: Base<T>,
    channel: (watch::Sender<Option<T>>, watch::Receiver<Option<T>>),
    node_id: NodeId,
    parent_node_id: Option<NodeId>,
    name: String,
    manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    subscriptions: Arc<SubscriptionCache>,
    ns: u16,
}

impl<T: Clone + Copy + Sync + Send + 'static + std::fmt::Debug + TypeName + ToDataTypeId>
    SlotInterface<T>
where
    opcua::types::Variant: OpcuaSupportedTypes<T>,
    opcua::types::Variant: From<T>,
{
    pub fn new(
        base: Base<T>,
        channel: (watch::Sender<Option<T>>, watch::Receiver<Option<T>>),
        manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
        subscriptions: Arc<SubscriptionCache>,
        ns: u16,
    ) -> Self {
        let name = base.name.clone();
        let node_id_name = format!("Slot/{}", Base::<T>::type_and_name(&base.name));
        Self {
            base,
            channel,
            node_id: NodeId::new(ns, node_id_name),
            parent_node_id: None,
            name,
            manager,
            subscriptions,
            ns,
        }
    }
    pub fn node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = node_id;
        self
    }
    pub fn parent_node(mut self, parent_node: NodeId) -> Self {
        self.parent_node_id = Some(parent_node);
        self
    }
    pub fn name(mut self, name: String) -> Self {
        self.name = name;
        self
    }
    pub fn register(mut self) {
        if self.parent_node_id.is_none() {
            let tfc_folder_id = NodeId::new(self.ns, "TFC");
            let slot_folder_id = NodeId::new(self.ns, "Slot");
            let type_folder_id = NodeId::new(self.ns, format!("Slot/{}", T::type_name()));
            {
                let mut address_space = self.manager.address_space().write();
                address_space.add_folder(
                    &tfc_folder_id,
                    "TFC",
                    "TFC",
                    &NodeId::objects_folder_id(),
                );
                address_space.add_folder(&slot_folder_id, "Slot", "Slot", &tfc_folder_id);
                address_space.add_folder(
                    &type_folder_id,
                    T::type_name(),
                    T::type_name(),
                    &slot_folder_id,
                );
            }
            self.parent_node_id = Some(type_folder_id);
        }

        let address_space = self.manager.address_space();
        {
            let mut address_space = address_space.write();
            VariableBuilder::new(&self.node_id, &self.name, &self.name)
                .data_type(T::to_data_type_id())
                .writable()
                .organized_by(self.parent_node_id.as_ref().unwrap())
                .insert(&mut address_space);
        }
        self.manager
            .inner()
            .add_write_callback(self.node_id.clone(), move |value, _| {
                if value.value.is_none() {
                    return opcua::types::StatusCode::BadNoValue;
                }

                let val: Result<T, Box<dyn std::error::Error + 'static>> =
                    value.value.unwrap().from_variant();
                if let Ok(v) = val {
                    self.channel.0.send(Some(v)).expect("Failed to send value");
                } else {
                    warn!("Unable to convert value: {:?}", val.err());
                }

                opcua::types::StatusCode::Good
            });
        let mut watch = self.channel.1;
        tokio::spawn(async move {
            loop {
                let val = *watch.borrow_and_update();
                match val {
                    Some(val) => {
                        self.manager
                            .set_value(
                                &self.subscriptions,
                                &self.node_id,
                                None,
                                DataValue::new_now(val),
                            )
                            .expect("Failed to set value");
                    }
                    None => {
                        warn!(target: &self.base.log_key, "opc ua got None value from slot");
                    }
                }
                watch.changed().await.expect("Failed to wait for change");
            }
        });
    }
}

pub trait ToDataTypeId {
    fn to_data_type_id() -> DataTypeId;
}

macro_rules! impl_to_data_type_id {
    ($(($type:ty, $variant:expr)),+ $(,)?) => {
        $(
            impl ToDataTypeId for $type {
                fn to_data_type_id() -> DataTypeId {
                    $variant
                }
            }
        )+
    };
}

impl_to_data_type_id!(
    (bool, DataTypeId::Boolean),
    (i8, DataTypeId::SByte),
    (u8, DataTypeId::Byte),
    (i16, DataTypeId::Int16),
    (u16, DataTypeId::UInt16),
    (i32, DataTypeId::Int32),
    (u32, DataTypeId::UInt32),
    (i64, DataTypeId::Int64),
    (u64, DataTypeId::UInt64),
    (f32, DataTypeId::Float),
    (f64, DataTypeId::Double),
    (String, DataTypeId::String),
);

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
