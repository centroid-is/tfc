use log::{error, log, trace, Level};
use schemars::JsonSchema;
use tokio::sync::watch;
use zbus::interface;

use crate::ipc::{Base, TypeName};

pub struct SignalInterface<T> {
    value_receiver: watch::Receiver<Option<T>>,
    log_key: String,
    watch_task: tokio::task::JoinHandle<()>,
}

impl<
        T: detail::SupportedTypes
            + zbus::zvariant::Type
            + Sync
            + Send
            + 'static
            + Clone
            + JsonSchema
            + for<'a> zbus::export::serde::de::Deserialize<'a>
            + zbus::export::serde::ser::Serialize
            + TypeName,
    > SignalInterface<T>
{
    pub fn register(base: Base<T>, dbus: zbus::Connection, watch: watch::Receiver<Option<T>>) {
        tokio::spawn(async move {
            Self::async_register(base, dbus, watch)
                .await
                .expect("Error registering signal");
        });
    }

    pub async fn async_register(
        base: Base<T>,
        dbus: zbus::Connection,
        watch: watch::Receiver<Option<T>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let path = format!(
            "/is/centroid/Signal/{}",
            Base::<T>::type_and_name(&base.name)
        );

        let mut watch_cp = watch.clone();
        let dbus_cp = dbus.clone();
        let path_cp = path.clone();
        let watch_task = tokio::spawn(async move {
            loop {
                let iface = dbus_cp
                    .clone()
                    .object_server()
                    .interface(path_cp.clone())
                    .await;
                if iface.is_err() {
                    trace!(
                        "Error getting interface: {} probably not started yet",
                        path_cp
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    continue;
                }
                let iface: zbus::object_server::InterfaceRef<SignalInterface<T>> = iface.unwrap();
                watch_cp
                    .changed()
                    .await
                    .expect("Error watching for changes");
                // Copy the value to prevent holding onto a borrow across an await point
                let copy_val: Option<T> = watch_cp.borrow_and_update().clone();
                match copy_val {
                    Some(ref value) => {
                        SignalInterface::value(&iface.signal_emitter(), value)
                            .await
                            .expect("Error sending value");
                    }
                    None => {
                        error!(target: &path_cp, "No value set");
                    }
                }
            }
        });

        let interface = Self {
            value_receiver: watch,
            log_key: path.clone(),
            watch_task,
        };
        dbus.object_server().at(path, interface).await?;

        Ok(())
    }
}

#[interface(name = "is.centroid.Signal")]
impl<T> SignalInterface<T>
where
    T: Clone
        + detail::SupportedTypes
        + zbus::zvariant::Type
        + Sync
        + Send
        + 'static
        + JsonSchema
        + for<'a> zbus::export::serde::de::Deserialize<'a>
        + zbus::export::serde::ser::Serialize,
{
    #[zbus(property, name = "Value")]
    fn value_prop(&self) -> Result<zbus::zvariant::Value<'static>, zbus::fdo::Error> {
        let ref_val = self.value_receiver.borrow();

        match *ref_val {
            Some(ref value) => Ok(value.to_value()), // this copies but is unfrequent
            None => Err(zbus::fdo::Error::Failed("No value set".into())),
        }
    }
    #[zbus(property(emits_changed_signal = "const"))]
    fn schema(&self) -> Result<String, zbus::fdo::Error> {
        serde_json::to_string_pretty(&schemars::schema_for!(T)).map_err(|e| {
            let err_msg = format!("Error serializing to JSON schema: {}", e);
            log!(target: &self.log_key, Level::Error, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })
    }
    fn last_value(&self) -> Result<T, zbus::fdo::Error> {
        let ref_val = self.value_receiver.borrow();

        match *ref_val {
            Some(ref value) => Ok(value.clone()),
            None => Err(zbus::fdo::Error::Failed("No value set".into())),
        }
    }
    #[zbus(signal)]
    async fn value(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        val: &T,
    ) -> Result<(), zbus::Error>;
}

impl<T> Drop for SignalInterface<T> {
    fn drop(&mut self) {
        self.watch_task.abort();
    }
}

pub struct SlotInterface<T> {
    value_sender: watch::Sender<Option<T>>,
    value_receiver: watch::Receiver<Option<T>>,
    log_key: String,
    watch_task: tokio::task::JoinHandle<()>,
}

impl<
        T: Clone
            + detail::SupportedTypes
            + zbus::zvariant::Type
            + Sync
            + Send
            + 'static
            + JsonSchema
            + for<'a> zbus::export::serde::de::Deserialize<'a>
            + zbus::export::serde::ser::Serialize
            + TypeName,
    > SlotInterface<T>
{
    pub fn register(
        base: Base<T>,
        dbus: zbus::Connection,
        channel: (watch::Sender<Option<T>>, watch::Receiver<Option<T>>),
    ) {
        tokio::spawn(async move {
            Self::async_register(base, dbus, channel)
                .await
                .expect("Error registering slot");
        });
    }

    pub async fn async_register(
        base: Base<T>,
        dbus: zbus::Connection,
        channel: (watch::Sender<Option<T>>, watch::Receiver<Option<T>>),
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let path = format!("/is/centroid/Slot/{}", Base::<T>::type_and_name(&base.name));
        let (value_sender, value_receiver) = channel;

        let mut watch_cp = value_receiver.clone();
        let dbus_cp = dbus.clone();
        let path_cp = path.clone();
        let watch_task = tokio::spawn(async move {
            loop {
                watch_cp
                    .changed()
                    .await
                    .expect("Error watching for changes");
                // Copy the value to prevent holding onto a borrow across an await point
                let copy_val: Option<T> = watch_cp.borrow_and_update().clone();
                match copy_val {
                    Some(ref value) => {
                        let iface: zbus::object_server::InterfaceRef<SlotInterface<T>> = dbus_cp
                            .clone()
                            .object_server()
                            .interface(path_cp.clone())
                            .await
                            .expect("Error getting interface");
                        SlotInterface::value(&iface.signal_emitter(), value)
                            .await
                            .expect("Error sending value");
                    }
                    None => {
                        error!(target: &path_cp, "No value set");
                    }
                }
            }
        });

        let interface = Self {
            value_sender,
            value_receiver,
            log_key: path.clone(),
            watch_task,
        };
        dbus.object_server().at(path, interface).await?;

        Ok(())
    }
}

#[interface(name = "is.centroid.Slot")]
impl<T> SlotInterface<T>
where
    T: Clone
        + detail::SupportedTypes
        + zbus::zvariant::Type
        + Sync
        + Send
        + 'static
        + JsonSchema
        + for<'a> zbus::export::serde::de::Deserialize<'a>
        + zbus::export::serde::ser::Serialize,
{
    #[zbus(property, name = "Value")]
    fn value_prop(&self) -> Result<zbus::zvariant::Value<'static>, zbus::fdo::Error> {
        let value = self.value_receiver.borrow();
        if let Some(ref value) = *value {
            return Ok(value.to_value());
        }
        Err(zbus::fdo::Error::Failed("No value set".into()))
    }
    #[zbus(property(emits_changed_signal = "const"))]
    fn schema(&self) -> Result<String, zbus::fdo::Error> {
        serde_json::to_string_pretty(&schemars::schema_for!(T)).map_err(|e| {
            let err_msg = format!("Error serializing to JSON schema: {}", e);
            log!(target: &self.log_key, Level::Error, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })
    }
    fn last_value(&self) -> Result<T, zbus::fdo::Error> {
        match *self.value_receiver.borrow() {
            Some(ref value) => Ok(value.clone()),
            None => Err(zbus::fdo::Error::Failed("No value set".into())),
        }
    }
    fn tinker(&mut self, value: T) -> Result<(), zbus::fdo::Error> {
        self.value_sender.send(Some(value)).map_err(|e| {
            let err_msg = format!("Error sending new value: {}", e);
            log!(target: &self.log_key, Level::Error, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })?;
        Ok(())
    }
    #[zbus(signal)]
    async fn value(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        val: &T,
    ) -> zbus::Result<()>;
}

impl<T> Drop for SlotInterface<T> {
    fn drop(&mut self) {
        self.watch_task.abort();
    }
}

mod detail {
    pub trait SupportedTypes {
        fn to_value(&self) -> zbus::zvariant::Value<'static>;
    }
    // make macro to impl for all types
    macro_rules! impl_supported_types {
    ($($t:ty),*) => {
        $(
            impl SupportedTypes for $t {
                fn to_value(&self) -> zbus::zvariant::Value<'static> {
                    zbus::zvariant::Value::from(*self)
                }
            }
        )*
    };
}
    impl_supported_types!(bool, i64, u64, f64);
    impl SupportedTypes for String {
        fn to_value(&self) -> zbus::zvariant::Value<'static> {
            zbus::zvariant::Value::from(self.clone())
        }
    }
}
