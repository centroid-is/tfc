use crate::devices::beckhoff::{ek1xxx::Ek1100, el1xxx::*, el2xxx::*, el3356::*};
use crate::devices::device_trait::{Device, DeviceInfo, UnimplementedDevice};
use crate::devices::lenze::i550::I550;
use log::warn;

pub fn make_device(
    dbus: zbus::Connection,
    vendor_id: u32,
    product_id: u32,
    slave_number: u16,
    alias_address: u16,
    name: &str,
) -> Box<dyn Device + Send + Sync> {
    match (vendor_id, product_id) {
        (Ek1100::VENDOR_ID, Ek1100::PRODUCT_ID) => Box::new(Ek1100),
        (El1002Info::VENDOR_ID, El1002Info::PRODUCT_ID) => {
            Box::new(El1002::new(dbus, slave_number, alias_address))
        }
        (El1008Info::VENDOR_ID, El1008Info::PRODUCT_ID) => {
            Box::new(El1008::new(dbus, slave_number, alias_address))
        }
        (El1809Info::VENDOR_ID, El1809Info::PRODUCT_ID) => {
            Box::new(El1809::new(dbus, slave_number, alias_address))
        }
        (El2794Info::VENDOR_ID, El2794Info::PRODUCT_ID) => {
            Box::new(El2794::new(dbus, slave_number, alias_address))
        }
        (El2004Info::VENDOR_ID, El2004Info::PRODUCT_ID) => {
            Box::new(El2004::new(dbus, slave_number, alias_address))
        }
        (El2008Info::VENDOR_ID, El2008Info::PRODUCT_ID) => {
            Box::new(El2008::new(dbus, slave_number, alias_address))
        }
        (El2809Info::VENDOR_ID, El2809Info::PRODUCT_ID) => {
            Box::new(El2809::new(dbus, slave_number, alias_address))
        }
        (I550::VENDOR_ID, I550::PRODUCT_ID) => {
            Box::new(I550::new(dbus, slave_number, alias_address))
        }
        (El3356::VENDOR_ID, El3356::PRODUCT_ID) => {
            Box::new(El3356::new(dbus, slave_number, alias_address))
        }
        _ => {
            warn!("Unimplemented device {name}: {vendor_id}:{product_id}");
            Box::new(UnimplementedDevice)
        }
    }
}
