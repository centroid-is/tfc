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
use std::error::Error;
use std::marker::PhantomData;
use std::sync::Arc;
use tfc::ipc::{Base, Signal};

pub type El1002 = El1xxx<El1002Info, 2, 1>;
pub type El1008 = El1xxx<El1008Info, 8, 1>;
pub type El1809 = El1xxx<El1809Info, 16, 2>;

// todo use this: https://github.com/rust-lang/rust/issues/76560
pub struct El1xxx<D: DeviceInfo + Entries<N>, const N: usize, const ARR_LEN: usize> {
    signals: [Signal<bool>; N],
    last_bits: [Option<bool>; N],
    log_key: String,
    _marker: PhantomData<D>,
    error: bool,
}

impl<D: DeviceInfo + Entries<N>, const N: usize, const ARR_LEN: usize> El1xxx<D, N, ARR_LEN> {
    pub fn new(dbus: zbus::Connection, subdevice_number: u16, _subdevice_alias: u16) -> Self {
        let log_key = format!("{}:{}", D::NAME, subdevice_number);
        let mut prefix = format!("{}/{subdevice_number}", D::NAME);
        if _subdevice_alias != 0 {
            prefix = format!("{}/alias/{_subdevice_alias}", D::NAME);
        }
        Self {
            signals: core::array::from_fn(|idx| {
                let signal = Signal::new(
                    dbus.clone(),
                    Base::new(format!("{prefix}/I{}", D::ENTRIES[idx]).as_str(), None),
                );
                #[cfg(feature = "dbus-expose")]
                tfc::ipc::dbus::SignalInterface::register(
                    signal.base(),
                    dbus.clone(),
                    signal.subscribe(),
                );
                signal
            }),
            last_bits: [None; N],
            log_key,
            _marker: PhantomData,
            error: false,
        }
    }
}

#[async_trait]
impl<D: DeviceInfo + Entries<N> + Send + Sync, const N: usize, const ARR_LEN: usize> Device
    for El1xxx<D, N, ARR_LEN>
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
        let input_data = device.inputs_raw();

        if input_data.len() != ARR_LEN && !self.error {
            error!(
                "Input data length mismatch: {} != {}",
                input_data.len(),
                ARR_LEN
            );
            self.error = true;
            return Err("Input data length mismatch".into());
        }
        self.error = false;

        let input_bits = input_data.view_bits::<bitvec::order::Lsb0>();

        for idx in 0..N {
            let bit = input_bits[idx];
            if self.last_bits[idx].is_none() || self.last_bits[idx].unwrap() != bit {
                let _ = self.signals[idx].async_send(bit).await.map_err(|e| {
                    error!("Error sending signal {}: {}", self.log_key, e);
                    e
                });
            }
            self.last_bits[idx] = Some(bit);
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
        for signal in self.signals.iter() {
            tfc::ipc::opcua::SignalInterface::new(
                signal.base(),
                signal.subscribe(),
                manager.clone(),
                subscriptions.clone(),
                namespace,
            )
            .register();
        }
        Ok(())
    }
}

pub struct El1002Info;
pub struct El1008Info;
pub struct El1809Info;

impl DeviceInfo for El1002Info {
    const VENDOR_ID: u32 = 0x2;
    const PRODUCT_ID: u32 = 0x3ea3052;
    const NAME: &'static str = "el1002";
}
impl DeviceInfo for El1008Info {
    const VENDOR_ID: u32 = 0x2;
    const PRODUCT_ID: u32 = 0x3f03052;
    const NAME: &'static str = "el1008";
}
impl DeviceInfo for El1809Info {
    const VENDOR_ID: u32 = 0x2;
    const PRODUCT_ID: u32 = 0x7113052;
    const NAME: &'static str = "el1809";
}

pub trait Entries<const N: usize> {
    const ENTRIES: [u8; N];
}
impl Entries<2> for El1002Info {
    const ENTRIES: [u8; 2] = [1, 5];
}
impl Entries<8> for El1008Info {
    const ENTRIES: [u8; 8] = [1, 5, 2, 6, 3, 7, 4, 8];
}
impl Entries<16> for El1809Info {
    const ENTRIES: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
}
