use async_trait::async_trait;
use atomic_refcell::AtomicRefMut;
use ethercrab::{SubDevice, SubDevicePdi, SubDeviceRef};
use std::error::Error;

#[async_trait]
pub trait Device {
    async fn setup<'maindevice, 'group>(
        &mut self,
        device: &mut SubDeviceRef<'maindevice, AtomicRefMut<'group, SubDevice>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;
    async fn process_data<'maindevice, 'group>(
        &mut self,
        device: &mut SubDeviceRef<'maindevice, SubDevicePdi<'group>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;
}

pub trait DeviceInfo {
    const VENDOR_ID: u32;
    const PRODUCT_ID: u32;
    const NAME: &'static str;
}

pub struct UnimplementedDevice;

#[async_trait]
impl Device for UnimplementedDevice {
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
        Ok(())
    }
}

pub trait Index {
    const INDEX: u16;
    const SUBINDEX: u8;
}

pub trait WriteValueIndex {
    async fn sdo_write_value_index<T>(&mut self, value: T) -> Result<(), ethercrab::error::Error>
    where
        T: Index + ethercrab_wire::EtherCrabWireWrite;
}

impl<SubDeviceType> WriteValueIndex for SubDeviceRef<'_, SubDeviceType>
where
    SubDeviceType: std::ops::Deref<Target = SubDevice>,
{
    async fn sdo_write_value_index<T>(&mut self, value: T) -> Result<(), ethercrab::error::Error>
    where
        T: Index + ethercrab_wire::EtherCrabWireWrite,
    {
        self.sdo_write(T::INDEX, T::SUBINDEX, value).await
    }
}

#[macro_export]
/// Define a value type for a device.
/// This macro defines a new struct with a single field of the given type.
/// The struct implements the `Index` trait and is marked as `#[derive(Default)]`.
macro_rules! define_value_type_internal {
    (
        $name:ident,
        $type:ty,
        $bytes:expr,
        $bits:expr,
        $default:expr,
        $index:expr,
        $subindex:expr
    ) => {
        #[derive(
            Debug,
            ethercrab_wire::EtherCrabWireWrite,
            serde::Serialize,
            serde::Deserialize,
            schemars::JsonSchema,
            Copy,
            Clone,
        )]
        #[wire(bytes = $bytes)]
        #[serde(transparent)]
        struct $name {
            #[wire(bits = $bits)]
            value: $type,
        }

        impl Default for $name {
            fn default() -> Self {
                Self { value: $default }
            }
        }

        impl Index for $name {
            const INDEX: u16 = $index;
            const SUBINDEX: u8 = $subindex;
        }
    };
}
#[macro_export]
/// Define a value type for a device.
/// This macro defines a new struct with a single field of the given type.
/// The struct implements the `Index` trait and is marked as `#[derive(Default)]`.
/// parameters:
/// - $name: the name of the struct
/// - $type: the type of the field
/// - $default: the default value of the field
/// - $index: the index of the SDO
/// - $subindex: the subindex of the SDO
macro_rules! define_value_type {
    ($name:ident, u16, $default:expr, $index:expr, $subindex:expr) => {
        $crate::define_value_type_internal!($name, u16, 2, 16, $default, $index, $subindex);
    };
    ($name:ident, u32, $default:expr, $index:expr, $subindex:expr) => {
        $crate::define_value_type_internal!($name, u32, 4, 32, $default, $index, $subindex);
    };
    ($name:ident, f32, $default:expr, $index:expr, $subindex:expr) => {
        $crate::define_value_type_internal!($name, f32, 4, 32, $default, $index, $subindex);
    };
}
