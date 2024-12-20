use crate::devices::device_trait::{Device, DeviceInfo};
use async_trait::async_trait;
use atomic_refcell::AtomicRefMut;
use ethercrab::{SubDevice, SubDevicePdi, SubDeviceRef};
#[cfg(feature = "opcua-expose")]
use opcua::server::{
    node_manager::memory::{InMemoryNodeManager, SimpleNodeManagerImpl},
    SubscriptionCache,
};
use std::error::Error;
use std::sync::Arc;

pub struct Ek1100;

#[async_trait]
impl Device for Ek1100 {
    async fn setup<'maindevice, 'group>(
        &mut self,
        device: &mut SubDeviceRef<'maindevice, AtomicRefMut<'group, SubDevice>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    async fn process_data<'maindevice, 'group>(
        &mut self,
        device: &mut SubDeviceRef<'maindevice, SubDevicePdi<'group>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    fn vendor_id(&self) -> u32 {
        Self::VENDOR_ID
    }
    fn product_id(&self) -> u32 {
        Self::PRODUCT_ID
    }
    #[cfg(feature = "opcua-expose")]
    fn opcua_register(
        &mut self,
        _: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
        _: Arc<SubscriptionCache>,
        _: u16,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
}

impl DeviceInfo for Ek1100 {
    const VENDOR_ID: u32 = 0x2;
    const PRODUCT_ID: u32 = 0x44c2c52;
    const NAME: &'static str = "Ek1100";
}
