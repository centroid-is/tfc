use futures::future::Either;
use futures::stream::StreamExt;
use futures::SinkExt;
use futures_channel::mpsc;
use log::{log, Level};
use quantities::mass::Mass;
use schemars::JsonSchema;
use std::marker::PhantomData;
use std::{error::Error, io, path::PathBuf, sync::Arc};
use tokio::select;
use tokio::sync::{Mutex, Notify, RwLock};
use zbus::{interface, zvariant::Type};
use zeromq::{
    PubSocket, Socket, SocketEvent, SocketRecv, SocketSend, SubSocket, ZmqError, ZmqMessage,
};

use crate::filter::AnyFilterDecl;
use crate::filter::Filter;
use crate::filter::Filters;
use crate::ipc_ruler_client::IpcRulerProxy;
use crate::progbase;

const FILE_PREFIX: &'static str = "ipc://";
const FILE_PATH: &'static str = "/var/run/tfc/";
fn endpoint(file_name: &str) -> String {
    format!("{}{}{}", FILE_PREFIX, FILE_PATH, file_name)
}

#[derive(Clone)]
pub struct Base<T> {
    pub name: String,
    pub description: Option<String>,
    pub value: Arc<RwLock<Option<T>>>,
    pub log_key: String,
}

impl<T: TypeName> Base<T> {
    pub fn new(name: &str, description: Option<String>) -> Self {
        Self {
            name: String::from(name),
            description,
            value: Arc::new(RwLock::new(None)),
            log_key: Self::type_and_name(name),
        }
    }
    fn type_and_name(name: &str) -> String {
        format!("{}/{}", T::type_name(), name)
    }
    fn full_name(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            progbase::exe_name(),
            progbase::proc_name(),
            T::type_name(),
            self.name
        )
    }
    fn endpoint(&self) -> String {
        endpoint(&self.full_name())
    }
    fn path(&self) -> PathBuf {
        PathBuf::from(format!("{}{}", FILE_PATH, self.full_name()))
    }
}

pub struct Slot<T>
where
    T: Send + Sync + 'static + PartialEq + AnyFilterDecl,
    <T as AnyFilterDecl>::Type: Filter<T>,
{
    slot: Arc<RwLock<SlotImpl<T>>>,
    last_value: Arc<Mutex<Option<T>>>,
    new_value_channel: Arc<Mutex<mpsc::Receiver<T>>>,
    cb: Option<Arc<Mutex<Box<dyn Fn(&T) + Send + Sync>>>>,
    connect_notify: Arc<Notify>,
    filters: Arc<Mutex<Filters<T>>>,
    dbus_path: String,
    bus: zbus::Connection,
    log_key: String,
}

impl<T> Slot<T>
where
    T: TypeName
        + TypeIdentifier
        + Deserialize
        + Send
        + Sync
        + 'static //
        + PartialEq
        // todo is this proper below
        + Type
        + Clone
        + detail::SupportedTypes
        + JsonSchema
        + for<'a> zbus::export::serde::de::Deserialize<'a>
        + zbus::export::serde::ser::Serialize
        + AnyFilterDecl,
    <T as AnyFilterDecl>::Type: Filter<T>,
{
    pub fn new(bus: zbus::Connection, base: Base<T>) -> Self
    where
        <T as AnyFilterDecl>::Type: Send
            + Sync
            + zbus::export::serde::ser::Serialize
            + for<'a> zbus::export::serde::de::Deserialize<'a>
            + JsonSchema,
    {
        let name = base.full_name();
        let description = base.description.clone().unwrap_or(String::new());
        let dbus_path = format!("/is/centroid/Slot/{}", Base::<T>::type_and_name(&base.name));
        let log_key = base.log_key.clone();

        let last_value: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
        let connect_notify = Arc::new(Notify::new());
        let filters = Arc::new(Mutex::new(Filters::new(
            bus.clone(),
            format!("filters/{}", Base::<T>::type_and_name(&base.name)).as_str(),
            Arc::clone(&last_value),
        )));
        let slot = Arc::new(RwLock::new(SlotImpl::new(base)));

        let log_key_cp = log_key.clone();
        let (new_value_sender, new_value_receiver) = mpsc::channel(10);
        let client = SlotInterface::new(Arc::clone(&last_value), new_value_sender, &log_key);
        let dbus_path_cp = dbus_path.clone();
        let bus_cp = bus.clone();

        let shared_slot: Arc<RwLock<SlotImpl<T>>> = Arc::clone(&slot);
        let shared_connect_notify = Arc::clone(&connect_notify);

        tokio::spawn(async move {
            // log if error
            let _ = bus_cp
                .object_server()
                .at(dbus_path_cp, client)
                .await
                .expect(&format!("Error registering object: {}", log_key_cp));
            let proxy = IpcRulerProxy::builder(&bus_cp)
                .cache_properties(zbus::CacheProperties::No)
                .build()
                .await
                .unwrap();
            loop {
                let res = proxy
                    .register_slot(name.as_str(), description.as_str(), T::type_identifier())
                    .await;
                if res.is_ok() {
                    break;
                } else {
                    log!(target: &log_key_cp, Level::Trace,
                    "Slot Registration failed: '{}', will try again in 1s", res.err().unwrap());
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                }
            }
            loop {
                let res = proxy.receive_connection_change().await;
                if res.is_ok() {
                    let args = res.unwrap().next().await.unwrap();
                    let slot_name = args.args().unwrap().slot_name().to_string();
                    if slot_name == name {
                        shared_connect_notify.notify_waiters();
                        // let _ = shared_slot.write().await.disconnect().await;
                        let signal_name = args.args().unwrap().signal_name().to_string();
                        log!(target: &log_key_cp, Level::Trace,
                            "Connection change received for slot: {}, signal: {}", slot_name, signal_name);
                        if signal_name.is_empty() {
                            continue;
                        }
                        let res = shared_slot.write().await.async_connect(&signal_name).await;
                        log!(target: &log_key_cp, Level::Trace,
                            "Connect result: {}", res.is_ok());
                        if res.is_err() {
                            log!(target: &log_key_cp, Level::Error,
                                "Failed to connect to signal: {}", res.err().unwrap());
                        }
                    }
                } else {
                }
            }
        });

        Self {
            slot,
            last_value,
            new_value_channel: Arc::new(Mutex::new(new_value_receiver)),
            cb: None,
            connect_notify,
            filters,
            dbus_path,
            bus,
            log_key,
        }
    }

    async fn process_value(
        value: T,
        shared_filters: Arc<Mutex<Filters<T>>>,
        shared_last_value: Arc<Mutex<Option<T>>>,
        shared_cb: Arc<Mutex<Box<dyn Fn(&T) + Send + Sync>>>,
        shared_bus: zbus::Connection,
        shared_dbus_path: &str,
        log_key: &str,
    ) {
        let filtered_value = shared_filters.lock().await.process(value).await;
        match filtered_value {
            Ok(filtered) => {
                *shared_last_value.lock().await = Some(filtered);
                let value_guard = shared_last_value.lock().await;
                let value_ref = value_guard.as_ref().unwrap();
                shared_cb.lock().await(value_ref);
                let iface: zbus::InterfaceRef<SlotInterface<T>> = shared_bus
                    .object_server()
                    .interface(shared_dbus_path)
                    .await
                    .unwrap();
                let _ = SlotInterface::value(&iface.signal_context(), value_ref).await;
            }
            Err(err) => {
                log!(target: log_key, Level::Trace,
                    "Filtered out value reason: {}", err);
            }
        }
    }

    pub fn recv(&mut self, callback: Box<dyn Fn(&T) + Send + Sync>)
    where
        <T as AnyFilterDecl>::Type: Send + Sync + Filter<T>,
    {
        let shared_cb: Arc<Mutex<Box<dyn Fn(&T) + Send + Sync>>> = Arc::new(Mutex::new(callback));
        self.cb = Some(Arc::clone(&shared_cb));
        let log_key = self.log_key.clone();
        let shared_slot: Arc<RwLock<SlotImpl<T>>> = Arc::clone(&self.slot);
        let shared_last_value = Arc::clone(&self.last_value);
        let shared_connect_notify = Arc::clone(&self.connect_notify);
        let shared_filters = Arc::clone(&self.filters);
        let shared_new_value_channel = Arc::clone(&self.new_value_channel);
        let shared_bus = self.bus.clone();
        let shared_dbus_path = self.dbus_path.clone();

        tokio::spawn(async move {
            loop {
                let slot_guard = shared_slot.read().await; // Create a binding for the read guard
                let mut new_value_guard = shared_new_value_channel.lock().await;
                tokio::select! {
                    result = slot_guard.recv() => {
                        match result {
                            Ok(value) => {
                                Slot::process_value(
                                    value,
                                    Arc::clone(&shared_filters),
                                    Arc::clone(&shared_last_value),
                                    Arc::clone(&shared_cb),
                                    shared_bus.clone(),
                                    shared_dbus_path.as_str(),
                                    &log_key,
                                ).await;
                            }
                            Err(e) => {
                                log!(target: &log_key, Level::Info,
                                    "Unsuccessful receive on slot, error: {}", e);
                            }
                        }
                    },
                    _ = shared_connect_notify.notified() => {
                        log!(target: &log_key, Level::Info,
                                "Connecting to other signal, let's try receiving from it");
                    },
                    result = new_value_guard.next() => {
                        match result {
                            Some(value) => {
                                Slot::process_value(
                                    value,
                                    Arc::clone(&shared_filters),
                                    Arc::clone(&shared_last_value),
                                    Arc::clone(&shared_cb),
                                    shared_bus.clone(),
                                    shared_dbus_path.as_str(),
                                    &log_key,
                                ).await;
                            }
                            None => {
                                log!(target: &log_key, Level::Trace,
                                    "New value channel ended");
                                break;
                            }
                        }
                    }
                }
            }
        });
    }
    pub fn connect(&mut self, signal_name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let shared_slot: Arc<RwLock<SlotImpl<T>>> = Arc::clone(&self.slot);
        let shared_connect_notify = Arc::clone(&self.connect_notify);
        let name = signal_name.to_string();
        tokio::spawn(async move {
            shared_connect_notify.notify_waiters();
            shared_slot.write().await.connect(&name)
        });
        Ok(())
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

struct SlotInterface<T> {
    last_value: Arc<Mutex<Option<T>>>,
    new_value_channel: mpsc::Sender<T>,
    log_key: String,
}

impl<T: detail::SupportedTypes + zbus::zvariant::Type + Sync + Send + 'static> SlotInterface<T> {
    pub fn new(
        last_value: Arc<Mutex<Option<T>>>,
        new_value_channel: mpsc::Sender<T>,
        key: &str,
    ) -> Self {
        Self {
            last_value,
            new_value_channel,
            log_key: key.to_string(),
        }
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
    async fn value_prop(&self) -> Result<zbus::zvariant::Value<'static>, zbus::fdo::Error> {
        let guard = self.last_value.lock().await;

        match *guard {
            Some(ref value) => Ok(value.to_value()),
            None => Err(zbus::fdo::Error::Failed("No value set".into())),
        }
    }
    #[zbus(property(emits_changed_signal = "const"))]
    async fn schema(&self) -> Result<String, zbus::fdo::Error> {
        serde_json::to_string_pretty(&schemars::schema_for!(T)).map_err(|e| {
            let err_msg = format!("Error serializing to JSON schema: {}", e);
            log!(target: &self.log_key, Level::Error, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })
    }
    async fn last_value(&self) -> Result<T, zbus::fdo::Error> {
        let guard = self.last_value.lock().await;
        match *guard {
            Some(ref value) => Ok(value.clone()),
            None => Err(zbus::fdo::Error::Failed("No value set".into())),
        }
    }
    async fn tinker(&mut self, value: T) -> Result<(), zbus::fdo::Error> {
        self.new_value_channel.send(value).await.map_err(|e| {
            let err_msg = format!("Error sending new value: {}", e);
            log!(target: &self.log_key, Level::Error, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })?;
        Ok(())
    }
    #[zbus(signal)]
    async fn value(signal_ctxt: &zbus::SignalContext<'_>, val: &T) -> zbus::Result<()>;
}

pub struct SlotImpl<T> {
    base: Base<T>,
    sock: Arc<Mutex<SubSocket>>,
    is_connected: Arc<Mutex<bool>>,
    connect_notify: Arc<Notify>,
}

impl<T> SlotImpl<T>
where
    T: TypeName + TypeIdentifier + Deserialize,
{
    pub fn new(base: Base<T>, // , callback: Box<dyn Fn(T)>
    ) -> Self {
        // todo: https://github.com/zeromq/zmq.rs/issues/196
        // let mut options = SocketOptions::default();
        // options.ZMQ_RECONNECT_IVL
        Self {
            base,
            sock: Arc::new(Mutex::new(SubSocket::new())),
            is_connected: Arc::new(Mutex::new(false)),
            connect_notify: Arc::new(Notify::new()),
        }
    }
    async fn async_connect_(
        log_key: &str,
        sock: Arc<Mutex<SubSocket>>,
        signal_name: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let socket_path = endpoint(signal_name);
        log!(target: &log_key, Level::Trace, "Trying to connect to: {}", socket_path);
        sock.lock().await.connect(socket_path.as_str()).await?;
        sock.lock().await.subscribe("").await?;
        Ok(())
    }
    pub async fn async_connect(
        &mut self,
        signal_name: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // todo reconnect
        let res = Self::async_connect_(
            self.base.log_key.as_str(),
            Arc::clone(&self.sock),
            signal_name,
        )
        .await;
        if res.is_ok() {
            *self.is_connected.lock().await = true;
            self.connect_notify.notify_waiters();
        }
        res
    }
    pub fn connect(&mut self, signal_name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let shared_sock = Arc::clone(&self.sock);
        let signal_name_str = signal_name.to_string();
        let log_key_cp = self.base.log_key.clone();
        tokio::spawn(async move {
            loop {
                let connect_task = async {
                    Self::async_connect_(
                        log_key_cp.as_str(),
                        Arc::clone(&shared_sock),
                        signal_name_str.as_str(),
                    )
                    .await
                };
                let timeout_task =
                    async { tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await };

                match select! {
                    result = connect_task => Either::Left(result),
                    _ = timeout_task => Either::Right(()),
                } {
                    Either::Left(Ok(_)) => {
                        log!(target: &log_key_cp, Level::Trace,
                            "Connect to: {:?} succesful", signal_name_str);
                        break;
                    }
                    Either::Left(Err(e)) => {
                        log!(target: &log_key_cp, Level::Trace, "Connect failed: {:?} will try again", e);
                    }
                    Either::Right(_) => {
                        log!(target: &log_key_cp, Level::Trace, "Connect timed out");
                    }
                }
            }
        });
        Ok(())
    }
    pub async fn recv(&self) -> Result<T, Box<dyn Error + Send + Sync>> {
        if !*self.is_connected.lock().await {
            log!(target: self.base.log_key.as_str(), Level::Trace, "Still not connected will wait until then");
            self.connect_notify.notified().await;
        }
        let buffer: ZmqMessage = self.sock.lock().await.recv().await?;
        // todo remove copying
        let flattened_buffer: Vec<u8> = buffer.iter().flat_map(|b| b.to_vec()).collect();
        let mut cursor = io::Cursor::new(flattened_buffer);
        let deserialized_packet =
            DeserializePacket::<T>::deserialize(&mut cursor).expect("Deserialization failed");
        Ok(deserialized_packet.value)
    }
}

pub struct Signal<T> {
    signal: Arc<Mutex<SignalImpl<T>>>,
    base: Base<T>,
    dbus_path: String,
    bus: zbus::Connection,
    log_key: String,
}

impl<T> Signal<T>
where
    T: TypeName
        + TypeIdentifier
        + SerializeSize
        + Serialize
        + Deserialize
        + Send
        + Sync
        + 'static //
        // todo is this proper below
        + Type
        + Clone
        + detail::SupportedTypes
        + JsonSchema
        + std::fmt::Debug
        + for<'a> zbus::export::serde::de::Deserialize<'a>
        + zbus::export::serde::ser::Serialize,
{
    pub fn new(bus: zbus::Connection, base: Base<T>) -> Self {
        let log_key_cp = base.log_key.clone();
        let log_key = base.log_key.clone();
        let client = SignalInterface::new(Arc::clone(&base.value), &log_key);
        let dbus_path = format!(
            "/is/centroid/Signal/{}",
            Base::<T>::type_and_name(&base.name)
        );
        let dbus_path_cp = dbus_path.clone();
        let bus_cp = bus.clone();
        let base_cp = base.clone();
        let signal = Arc::new(Mutex::new(SignalImpl::new(base_cp)));
        let signal_cp = Arc::clone(&signal);

        let name = base.full_name();
        let description = base.description.clone().unwrap_or(String::new());
        tokio::spawn(async move {
            let _ = signal_cp.lock().await.init().await;
            let _ = bus_cp
                .object_server()
                .at(dbus_path_cp, client)
                .await
                .expect(&format!("Error registering object: {}", log_key_cp));
            let proxy = IpcRulerProxy::builder(&bus_cp)
                .cache_properties(zbus::CacheProperties::No)
                .build()
                .await
                .unwrap();
            loop {
                let res = proxy
                    .register_signal(name.as_str(), description.as_str(), T::type_identifier())
                    .await;
                if res.is_ok() {
                    break;
                } else {
                    log!(target: &log_key_cp, Level::Trace,
                "Signal Registration failed: '{}', will try again in 1s", res.err().unwrap());
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                }
            }
        });
        Self {
            signal,
            base,
            dbus_path,
            bus,
            log_key,
        }
    }
    pub async fn send(&mut self, value: T) -> Result<(), Box<dyn Error>> {
        self.signal.lock().await.send(value).await?;
        let value_guard = self.base.value.read().await;
        let value_ref = value_guard.as_ref().unwrap();
        let iface: zbus::InterfaceRef<SignalInterface<T>> = self
            .bus
            .object_server()
            .interface(self.dbus_path.as_str())
            .await
            .unwrap();
        SignalInterface::value(&iface.signal_context(), value_ref).await?;
        Ok(())
    }
    pub fn full_name(&self) -> String {
        self.base.full_name()
    }
}

struct SignalInterface<T> {
    last_value: Arc<RwLock<Option<T>>>,
    log_key: String,
}

impl<T: detail::SupportedTypes + zbus::zvariant::Type + Sync + Send + 'static> SignalInterface<T> {
    pub fn new(last_value: Arc<RwLock<Option<T>>>, key: &str) -> Self {
        Self {
            last_value,
            log_key: key.to_string(),
        }
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
    async fn value_prop(&self) -> Result<zbus::zvariant::Value<'static>, zbus::fdo::Error> {
        let guard = self.last_value.read().await;

        match *guard {
            Some(ref value) => Ok(value.to_value()),
            None => Err(zbus::fdo::Error::Failed("No value set".into())),
        }
    }
    #[zbus(property(emits_changed_signal = "const"))]
    async fn schema(&self) -> Result<String, zbus::fdo::Error> {
        serde_json::to_string_pretty(&schemars::schema_for!(T)).map_err(|e| {
            let err_msg = format!("Error serializing to JSON schema: {}", e);
            log!(target: &self.log_key, Level::Error, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })
    }
    async fn last_value(&self) -> Result<T, zbus::fdo::Error> {
        let guard = self.last_value.read().await;
        match *guard {
            Some(ref value) => Ok(value.clone()),
            None => Err(zbus::fdo::Error::Failed("No value set".into())),
        }
    }
    #[zbus(signal)]
    async fn value(signal_ctxt: &zbus::SignalContext<'_>, val: &T) -> zbus::Result<()>;
}

pub struct SignalImpl<T> {
    base: Base<T>,
    sock: Arc<Mutex<PubSocket>>,
    monitor: Option<mpsc::Receiver<SocketEvent>>,
}

impl<T> SignalImpl<T>
where
    T: TypeName
        + TypeIdentifier
        + SerializeSize
        + Serialize
        + Deserialize
        + std::fmt::Debug
        + std::marker::Sync
        + std::marker::Send
        + 'static,
{
    pub fn new(base: Base<T>) -> Self {
        Self {
            base,
            sock: Arc::new(Mutex::new(PubSocket::new())),
            monitor: None,
        }
    }
    pub async fn init(&mut self) -> Result<(), Box<dyn Error>> {
        // Todo cpp azmq does this by itself
        // We should check whether any PID is binded to the file before removing it
        // But the bind below fails if the file exists, even though no one is binded to it
        let path = self.base.path();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        self.sock
            .lock()
            .await
            .bind(self.base.endpoint().as_str())
            .await?;
        self.monitor = Some(self.sock.lock().await.monitor());

        let shared_value = Arc::clone(&self.base.value);
        let shared_sock = Arc::clone(&self.sock);
        let log_key = self.base.log_key.clone();
        if let Some(mut receiver) = self.monitor.take() {
            tokio::spawn(async move {
                while let Some(event) = receiver.next().await {
                    match event {
                        SocketEvent::Accepted { .. } => {
                            log!(target: &log_key, Level::Trace,
                                "Accepted event, last value: {:?}", shared_value.read().await
                            );
                            let locked_sock = shared_sock.lock();
                            let mut buffer = Vec::new();
                            {
                                let value = shared_value.read().await;
                                if value.is_none() {
                                    return Ok(()) as Result<(), ZmqError>;
                                }
                                let packet = SerializePacket::new(value.as_ref().unwrap());
                                packet.serialize(&mut buffer).expect("Serialization failed");
                            }
                            // TODO use ZMQ_EVENT_HANDSHAKE_SUCCEEDED and throw this sleep out
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            locked_sock.await.send(ZmqMessage::from(buffer)).await?;
                        }
                        other_event => {
                            log!(target: &log_key, Level::Info,
                                "Other event: {:?}, last value: {:?}",
                                other_event,
                                shared_value.read().await
                            );
                        }
                    }
                }
                println!("Receiver has been dropped. Task terminating.");
                Ok(())
            });
        }
        Ok(())
    }
    pub async fn send(&mut self, value: T) -> Result<(), Box<dyn Error>> {
        let mut buffer = Vec::new();
        {
            let packet = SerializePacket::new(&value);
            packet.serialize(&mut buffer).expect("Serialization failed");
        }
        *self.base.value.write().await = Some(value);
        self.sock
            .lock()
            .await
            .send(ZmqMessage::from(buffer))
            .await?;
        Ok(())
    }

    pub fn full_name(&self) -> String {
        self.base.full_name()
    }
}

// ------------------ trait TypeName ------------------

pub trait TypeName {
    fn type_name() -> &'static str;
}

macro_rules! impl_type_name {
    ($($t:ty => $name:expr),*) => {
        $(
            impl TypeName for $t {
                fn type_name() -> &'static str {
                    $name
                }
            }
        )*
    };
}

impl_type_name! {
    bool => "bool",
    i64 => "i64",
    u64 => "u64",
    f64 => "double",
    String => "string",
    // todo json
    Mass => "mass"
}

// ------------------ trait TypeIdentifier ------------------

pub trait TypeIdentifier {
    fn type_identifier() -> u8;
}

macro_rules! impl_type_identifier {
    ($($t:ty => $name:expr),*) => {
        $(
            impl TypeIdentifier for $t {
                fn type_identifier() -> u8 {
                    $name
                }
            }
        )*
    };
}

impl_type_identifier! {
    bool => 1,
    i64 => 2,
    u64 => 3,
    f64 => 4,
    String => 5,
    // todo json
    Result<Mass, i32> => 7, // TODO let's just make a ipc_mass.rs or something and impl this there with its enum
    Mass => 7 // TODO can we deduce this somehow
}

// ------------------ enum Protocol Version ------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    Unknown,
    V0,
}

trait IsFundamental {}
macro_rules! impl_is_fundamental {
    ($($t:ty),*) => {
        $(
            impl IsFundamental for $t {}
        )*
    };
}
impl_is_fundamental!(bool, i64, i32, i16, i8, u64, u32, u16, u8, f64, f32);

trait ToLeBytesVec {
    fn to_le_bytes_vec(&self) -> Vec<u8>;
}
macro_rules! impl_to_le_bytes_vec_for_fundamental {
    ($($t:ty),*) => {
        $(
            impl ToLeBytesVec for $t {
                fn to_le_bytes_vec(&self) -> Vec<u8> {
                    self.to_le_bytes().to_vec()
                }
            }
        )*
    };
}
impl_to_le_bytes_vec_for_fundamental!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);
impl ToLeBytesVec for bool {
    fn to_le_bytes_vec(&self) -> Vec<u8> {
        vec![*self as u8]
    }
}

trait SerializeSize {
    fn serialize_size(&self) -> usize;
}
impl<T: IsFundamental> SerializeSize for T {
    fn serialize_size(&self) -> usize {
        std::mem::size_of::<T>()
    }
}
impl SerializeSize for String {
    fn serialize_size(&self) -> usize {
        self.len()
    }
}
impl<T, E> SerializeSize for Result<T, E>
where
    T: quantities::Quantity,
    E: IsFundamental,
{
    fn serialize_size(&self) -> usize {
        1 // Used to indicate whether the Result is Type or Error
            + if self.is_ok() { std::mem::size_of::<quantities::AmountT>() } else { std::mem::size_of::<E>() }
    }
}
trait Serialize {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()>;
}
impl<T> Serialize for T
where
    T: ToLeBytesVec,
{
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&self.to_le_bytes_vec())?;
        Ok(())
    }
}
impl Serialize for String {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&self.as_bytes())?;
        Ok(())
    }
}
impl<T, E> Serialize for Result<T, E>
where
    T: quantities::Quantity,
    E: IsFundamental + std::fmt::Debug + ToLeBytesVec,
{
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        if self.is_ok() {
            writer.write_all(&[1 as u8])?; // use 1 to indicate value being okay
            writer.write_all(&self.as_ref().unwrap().amount().to_le_bytes())?;
        } else {
            writer.write_all(&[0 as u8])?; // use 1 to indicate value being okay
            writer.write_all(&self.as_ref().err().unwrap().to_le_bytes_vec())?;
        }
        Ok(())
    }
}

trait FromLeBytes: Sized {
    fn from_le_bytes(bytes: &[u8]) -> Self;
}
macro_rules! impl_from_le_bytes_for_fundamental {
    ($($t:ty),*) => {
        $(
            impl FromLeBytes for $t {
                fn from_le_bytes(bytes: &[u8]) -> Self {
                    assert!(bytes.len() == std::mem::size_of::<$t>(), "Invalid byte slice length for {}", stringify!($t));
                    let mut arr = [0u8; std::mem::size_of::<$t>()];
                    arr.copy_from_slice(bytes);
                    <$t>::from_le_bytes(arr)
                }
            }
        )*
    };
}
impl_from_le_bytes_for_fundamental!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);
impl FromLeBytes for bool {
    fn from_le_bytes(bytes: &[u8]) -> Self {
        assert!(bytes.len() == 1, "Invalid byte slice length for bool");
        bytes[0] != 0
    }
}

pub trait Deserialize: Sized {
    fn deserialize<R: io::Read>(reader: &mut R) -> io::Result<Self>;
}

impl<T> Deserialize for T
where
    T: FromLeBytes,
{
    fn deserialize<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let mut buffer = vec![0u8; std::mem::size_of::<T>()];
        reader.read_exact(&mut buffer)?;
        Ok(T::from_le_bytes(&buffer))
    }
}

impl Deserialize for String {
    fn deserialize<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer)?;
        String::from_utf8(buffer).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

impl<T, E> Deserialize for Result<T, E>
where
    T: quantities::Quantity<UnitType = quantities::mass::MassUnit>
        // TODO this now only supports mass
        + TypeIdentifier,
    quantities::AmountT: FromLeBytes, // I would much rather like T::AmountT
    E: IsFundamental + std::fmt::Debug + FromLeBytes,
{
    fn deserialize<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let mut status_byte = [0u8; 1];
        reader.read_exact(&mut status_byte)?;

        if status_byte[0] == 1 {
            // Deserialize as Ok(T)
            let mut amount_buffer = [0u8; std::mem::size_of::<quantities::AmountT>()];
            reader.read_exact(&mut amount_buffer)?;
            let amount = quantities::AmountT::from_le_bytes(amount_buffer);
            // So the unit type of Quantity is a member variable so we cannot determine the type
            // at compile time, and this is god damn bloated
            let unit;
            if T::type_identifier() == Mass::type_identifier() {
                unit = quantities::mass::MILLIGRAM;
            } else {
                panic!(
                    "Unable to determine UnitType for type id: {}",
                    T::type_identifier()
                )
            }
            Ok(Ok(T::new(amount, unit)))
        } else if status_byte[0] == 0 {
            // Deserialize as Err(E)
            let mut error_buffer = vec![0u8; std::mem::size_of::<E>()];
            reader.read_exact(&mut error_buffer)?;
            let error = E::from_le_bytes(&error_buffer);
            Ok(Err(error))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid status byte in serialized Result",
            ))
        }
    }
}

#[derive(Debug, Clone)]
struct Header<T> {
    version: Version,
    type_id: u8,
    value_size: usize,
    _phantom: PhantomData<T>,
}

impl<T: TypeIdentifier> Header<T> {
    fn new(value_size: usize) -> Self {
        Self {
            version: Version::V0,
            type_id: T::type_identifier(),
            value_size,
            _phantom: PhantomData,
        }
    }

    fn size() -> usize {
        std::mem::size_of::<Version>() + std::mem::size_of::<u8>() + std::mem::size_of::<usize>()
    }

    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&[self.version as u8])?;
        writer.write_all(&[self.type_id as u8])?;
        writer.write_all(&self.value_size.to_le_bytes())?;
        Ok(())
    }

    fn deserialize<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let mut version_buf = [0u8; 1];
        reader.read_exact(&mut version_buf)?;
        let _ = match version_buf[0] {
            0 => Version::Unknown,
            1 => Version::V0,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid version",
                ))
            }
        };

        let mut type_id_buf = [0u8; 1];
        reader.read_exact(&mut type_id_buf)?;
        let type_id = type_id_buf[0];

        let mut size_buf = [0u8; std::mem::size_of::<usize>()];
        reader.read_exact(&mut size_buf)?;
        let value_size = usize::from_le_bytes(size_buf);

        if type_id != T::type_identifier() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid type"));
        }
        Ok(Self::new(value_size))
    }
}

#[derive(Debug)]
struct SerializePacket<'a, V> {
    header: Header<V>,
    value: &'a V,
}

impl<'a, V> SerializePacket<'a, V>
where
    V: TypeIdentifier + SerializeSize + Serialize + Deserialize,
{
    fn new(value: &'a V) -> Self {
        Self {
            header: Header::new(V::serialize_size(&value)),
            value,
        }
    }
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        self.header.serialize(writer)?;
        self.value.serialize(writer)?;
        Ok(())
    }
}

#[derive(Debug)]
struct DeserializePacket<V> {
    header: Header<V>,
    value: V,
}

impl<V> DeserializePacket<V>
where
    V: TypeIdentifier + Deserialize,
{
    fn deserialize<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let header = Header::deserialize(reader)?;
        let value = V::deserialize(reader)?;
        Ok(Self { header, value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantities::mass::Mass;
    use rand::Rng;
    use std::io::Cursor;

    fn packet_serialize_deserialize<T>(value: T)
    where
        T: TypeIdentifier + SerializeSize + Serialize + Deserialize + PartialEq + std::fmt::Debug,
    {
        let packet = SerializePacket::new(&value);

        // Serialize the packet
        let mut buffer = Vec::new();
        packet.serialize(&mut buffer).expect("Serialization failed");

        // Deserialize the packet
        let mut cursor = Cursor::new(buffer);
        let deserialized_packet =
            DeserializePacket::<T>::deserialize(&mut cursor).expect("Deserialization failed");

        // Check that the deserialized value matches the original
        assert_eq!(*packet.value, deserialized_packet.value);
        assert_eq!(
            packet.header.value_size,
            deserialized_packet.header.value_size
        );
        assert_eq!(packet.header.type_id, deserialized_packet.header.type_id);
    }

    #[test]
    fn test_packet_serialize_deserialize_fundamentals() {
        packet_serialize_deserialize(std::i64::MAX);
        packet_serialize_deserialize(std::i64::MIN);
        packet_serialize_deserialize(1337 as i64);

        packet_serialize_deserialize(std::u64::MAX);
        packet_serialize_deserialize(std::u64::MIN);
        packet_serialize_deserialize(1337 as u64);

        packet_serialize_deserialize(std::f64::MAX);
        packet_serialize_deserialize(std::f64::MIN);
        packet_serialize_deserialize(13.37 as f64);

        packet_serialize_deserialize(true);
        packet_serialize_deserialize(false);
    }

    #[test]
    fn test_packet_serialize_deserialize_string() {
        packet_serialize_deserialize("false".to_string());

        let size = 1024 * 1024; // 1KB * 1KB
        let mut rng = rand::thread_rng();
        let mut random_string = String::with_capacity(size);

        for _ in 0..size {
            let random_char = rng.gen_range('a'..='z');
            random_string.push(random_char);
        }

        packet_serialize_deserialize(random_string);
    }

    #[test]
    fn test_packet_serialize_deserialize_mass() {
        packet_serialize_deserialize(
            Ok(quantities::Amnt!(10.0) * quantities::mass::MILLIGRAM) as Result<Mass, i32>
        );
        packet_serialize_deserialize(
            Ok(quantities::Amnt!(1333333337) * quantities::mass::MILLIGRAM) as Result<Mass, i32>,
        );
        packet_serialize_deserialize(
            Ok(quantities::Amnt!(-1333333337) * quantities::mass::MILLIGRAM) as Result<Mass, i32>,
        );
        packet_serialize_deserialize(
            Ok(quantities::Amnt!(13333.33337) * quantities::mass::MILLIGRAM) as Result<Mass, i32>,
        );
        packet_serialize_deserialize(Ok(
            quantities::Amnt!(-13333.33337) * quantities::mass::MILLIGRAM
        ) as Result<Mass, i32>);

        packet_serialize_deserialize(Err(-1) as Result<Mass, i32>); // todo make
    }
}
