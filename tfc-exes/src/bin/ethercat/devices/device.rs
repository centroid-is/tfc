use crate::devices::beckhoff::{ek1xxx::Ek1100, el1xxx::*};
use crate::devices::device_trait::{Device, DeviceInfo, UnimplementedDevice};

pub fn make_device(
    dbus: zbus::Connection,
    vendor_id: u32,
    product_id: u32,
    slave_number: u16,
    alias_address: u16,
) -> Box<dyn Device> {
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
        _ => Box::new(UnimplementedDevice),
    }
}
