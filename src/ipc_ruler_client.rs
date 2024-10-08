use zbus::proxy;

#[cfg(feature = "s3x-legacy-naming")]
#[proxy(
    gen_async = true,
    interface = "com.skaginn3x.manager",
    default_service = "com.skaginn3x.ipc_ruler",
    default_path = "/com/skaginn3x/ipc_ruler"
)]
trait IpcRuler {
    #[zbus(property)]
    fn connections(&self) -> zbus::fdo::Result<String>;

    #[zbus(property)]
    fn signals(&self) -> zbus::fdo::Result<String>;

    #[zbus(property)]
    fn slots(&self) -> zbus::fdo::Result<String>;

    fn connect(&self, slot_name: &str, signal_name: &str) -> zbus::fdo::Result<()>;

    fn disconnect(&self, slot_name: &str) -> zbus::fdo::Result<()>;

    fn register_signal(&self, name: &str, description: &str, type_id: u8) -> zbus::fdo::Result<()>;

    fn register_slot(&self, name: &str, description: &str, type_id: u8) -> zbus::fdo::Result<()>;

    #[zbus(signal)]
    fn connection_change(&self, slot_name: &str, signal_name: &str) -> fdo::Result<()>;
}


#[cfg(not(feature = "s3x-legacy-naming"))]
#[proxy(
    gen_async = true,
    interface = "is.centroid.manager",
    default_service = "is.centroid.ipc_ruler",
    default_path = "/is/centroid/ipc_ruler"
)]
pub trait IpcRuler {
    #[zbus(property)]
    fn connections(&self) -> zbus::fdo::Result<String>;

    #[zbus(property)]
    fn signals(&self) -> zbus::fdo::Result<String>;

    #[zbus(property)]
    fn slots(&self) -> zbus::fdo::Result<String>;

    fn connect(&self, slot_name: &str, signal_name: &str) -> zbus::fdo::Result<()>;

    fn disconnect(&self, slot_name: &str) -> zbus::fdo::Result<()>;

    fn register_signal(&self, name: &str, description: &str, type_id: u8) -> zbus::fdo::Result<()>;

    fn register_slot(&self, name: &str, description: &str, type_id: u8) -> zbus::fdo::Result<()>;

    #[zbus(signal)]
    fn connection_change(&self, slot_name: &str, signal_name: &str) -> fdo::Result<()>;
}
