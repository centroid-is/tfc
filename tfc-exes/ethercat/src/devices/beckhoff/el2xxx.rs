use crate::devices::device_trait::{Device, DeviceInfo};
use async_trait::async_trait;
use atomic_refcell::AtomicRefMut;
use bitvec::view::BitView;
use ethercrab::{SubDevice, SubDevicePdi, SubDeviceRef};
use log::error;
#[cfg(feature = "opcua-expose")]
use opcua::server::{
    node_manager::memory::{InMemoryNodeManager, SimpleNodeManagerImpl},
    SubscriptionCache,
};
use std::marker::PhantomData;
use std::sync::Arc;
use std::{error::Error, sync::atomic::AtomicBool};
use tfc::ipc::{Base, Slot};

pub type El2794 = El2xxx<El2794Info, 4, 1>;
pub type El2004 = El2xxx<El2004Info, 4, 1>;
pub type El2008 = El2xxx<El2008Info, 8, 1>;
pub type El2809 = El2xxx<El2809Info, 16, 2>;

// todo use this: https://github.com/rust-lang/rust/issues/76560
pub struct El2xxx<D: DeviceInfo + Entries<N>, const N: usize, const ARR_LEN: usize> {
    slots: [Slot<bool>; N],
    last_bits: [Arc<AtomicBool>; N],
    _marker: PhantomData<D>,
    error: bool,
}

impl<D: DeviceInfo + Entries<N>, const N: usize, const ARR_LEN: usize> El2xxx<D, N, ARR_LEN> {
    pub fn new(dbus: zbus::Connection, subdevice_number: u16, _subdevice_alias: u16) -> Self {
        let last_bits = core::array::from_fn(|_| Arc::new(AtomicBool::new(false)));
        let mut prefix = format!("{}/{subdevice_number}", D::NAME);
        if _subdevice_alias != 0 {
            prefix = format!("{}/alias/{_subdevice_alias}", D::NAME);
        }
        Self {
            slots: core::array::from_fn(|idx| {
                let mut slot = Slot::new(
                    dbus.clone(),
                    Base::new(format!("{prefix}/in{}", D::ENTRIES[idx]).as_str(), None),
                );
                #[cfg(feature = "dbus-expose")]
                tfc::ipc::dbus::SlotInterface::register(
                    slot.base(),
                    dbus.clone(),
                    slot.channel("dbus"),
                );
                let last_bit = last_bits[idx].clone();
                slot.recv(Box::new(move |bit| {
                    last_bit.store(*bit, std::sync::atomic::Ordering::Relaxed);
                }));
                slot
            }),
            last_bits,
            // log_key,
            _marker: PhantomData,
            error: false,
        }
    }
}

#[async_trait]
impl<D: DeviceInfo + Entries<N> + Send + Sync, const N: usize, const ARR_LEN: usize> Device
    for El2xxx<D, N, ARR_LEN>
{
    async fn setup<'maindevice, 'group>(
        &mut self,
        _device: &mut SubDeviceRef<'maindevice, AtomicRefMut<'group, SubDevice>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    async fn process_data<'maindevice, 'group>(
        &mut self,
        device: &mut SubDeviceRef<'maindevice, SubDevicePdi<'group>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let output_data = device.outputs_raw_mut();

        if output_data.len() != ARR_LEN && !self.error {
            error!(
                "Output data length mismatch: {} != {}",
                output_data.len(),
                ARR_LEN
            );
            self.error = true;
            return Err("Output data length mismatch".into());
        }
        self.error = false;

        let output_bits = output_data.view_bits_mut::<bitvec::order::Lsb0>();

        for idx in 0..N {
            let bit = self.last_bits[idx].load(std::sync::atomic::Ordering::Relaxed);
            output_bits.set(idx, bit);
        }

        Ok(())
    }
    fn vendor_id(&self) -> u32 {
        D::VENDOR_ID
    }
    fn product_id(&self) -> u32 {
        D::PRODUCT_ID
    }
    #[cfg(feature = "opcua-expose")]
    fn opcua_register(
        &mut self,
        manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
        subscriptions: Arc<SubscriptionCache>,
        namespace: u16,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        for slot in self.slots.iter() {
            tfc::ipc::opcua::SlotInterface::new(
                slot.base(),
                slot.channel("opcua"),
                manager.clone(),
                subscriptions.clone(),
                namespace,
            )
            .register();
        }
        Ok(())
    }
}

pub struct El2794Info;
pub struct El2004Info;
pub struct El2008Info;
pub struct El2809Info;

impl DeviceInfo for El2794Info {
    const VENDOR_ID: u32 = 0x2;
    const PRODUCT_ID: u32 = 0xaea3052;
    const NAME: &'static str = "el2794";
}
impl DeviceInfo for El2004Info {
    const VENDOR_ID: u32 = 0x2;
    const PRODUCT_ID: u32 = 0x7d43052;
    const NAME: &'static str = "el2004";
}
impl DeviceInfo for El2008Info {
    const VENDOR_ID: u32 = 0x2;
    const PRODUCT_ID: u32 = 0x7d83052;
    const NAME: &'static str = "el2008";
}
impl DeviceInfo for El2809Info {
    const VENDOR_ID: u32 = 0x2;
    const PRODUCT_ID: u32 = 0xaf93052;
    const NAME: &'static str = "el2809";
}

pub trait Entries<const N: usize> {
    const ENTRIES: [u8; N];
}
impl Entries<4> for El2794Info {
    const ENTRIES: [u8; 4] = [1, 5, 4, 8];
}
impl Entries<4> for El2004Info {
    const ENTRIES: [u8; 4] = [1, 5, 4, 8];
}
impl Entries<8> for El2008Info {
    const ENTRIES: [u8; 8] = [1, 5, 2, 6, 3, 7, 4, 8];
}
impl Entries<16> for El2809Info {
    const ENTRIES: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
}
