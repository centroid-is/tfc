use futures::future::Either;
use futures::stream::StreamExt;
use futures::SinkExt;
use futures_channel::mpsc;
use log::{debug, log, trace, Level};
use parking_lot::Mutex;
use quantities::mass::Mass;
use schemars::JsonSchema;
use std::marker::PhantomData;
use std::{error::Error, io, path::PathBuf, sync::Arc};
use tokio::select;
use tokio::sync::Mutex as TMutex;
use tokio::sync::{watch, Notify, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use zbus::{interface, zvariant::Type};
use zeromq::{
    PubSocket, Socket, SocketEvent, SocketRecv, SocketSend, SubSocket, ZmqError, ZmqMessage,
};

use crate::filter::AnyFilterDecl;
use crate::filter::Filter;
use crate::filter::Filters;
use crate::ipc_ruler_client::IpcRulerProxy;
use crate::progbase;

pub mod dbus;

const FILE_PREFIX: &'static str = "ipc://";
const FILE_PATH: &'static str = "/var/run/tfc/";
fn endpoint(file_name: &str) -> String {
    format!("{}{}{}", FILE_PREFIX, FILE_PATH, file_name)
}

#[derive(Clone)]
pub struct Base<T> {
    pub name: String,
    pub description: Option<String>,
    pub log_key: String,
    _marker: PhantomData<T>,
}

impl<T: TypeName> Base<T> {
    pub fn new(name: &str, description: Option<&str>) -> Self {
        Self {
            name: String::from(name),
            description: description.map(|s| s.to_string()),
            log_key: Self::type_and_name(name),
            _marker: PhantomData,
        }
    }
    pub fn type_and_name(name: &str) -> String {
        format!("{}/{}", T::type_name(), name)
    }
    pub fn full_name(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            progbase::exe_name(),
            progbase::proc_name(),
            T::type_name(),
            self.name
        )
    }
    pub fn endpoint(&self) -> String {
        endpoint(&self.full_name())
    }
    pub fn path(&self) -> PathBuf {
        let runtime_dir =
            std::env::var("RUNTIME_DIRECTORY").unwrap_or_else(|_| FILE_PATH.to_string());
        PathBuf::from(format!("{}{}", runtime_dir, self.full_name()))
    }
}

pub struct Slot<T>
where
    T: Send + Sync + 'static + PartialEq + AnyFilterDecl,
    <T as AnyFilterDecl>::Type: Filter<T>,
{
    slot: Arc<RwLock<SlotImpl<T>>>,
    connect_notify: Arc<Notify>,
    log_key: String,
    new_value_signal: watch::Sender<Option<T>>,
    connect_change_task: JoinHandle<()>,
    register_task: JoinHandle<()>,
    recv_task: JoinHandle<()>,
    base: Base<T>,
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
        + JsonSchema
        + for<'a> zbus::export::serde::de::Deserialize<'a>
        + zbus::export::serde::ser::Serialize
        + AnyFilterDecl
        + std::fmt::Debug,
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
        let log_key = base.log_key.clone();

        let connect_notify = Arc::new(Notify::new());

        let slot = Arc::new(RwLock::new(SlotImpl::new(base.clone())));

        let (new_value_sender, new_value_receiver) = watch::channel(None as Option<T>);

        let shared_slot: Arc<RwLock<SlotImpl<T>>> = Arc::clone(&slot);
        let shared_connect_notify = Arc::clone(&connect_notify);

        // watch for connection changes for this slot
        // TODO we should make a awaitable boolean flag, we need to have started this spawn strictly before the next one where we register the slot

        let shared_log_key = log_key.clone();
        let shared_bus = bus.clone();
        let shared_name = base.full_name();

        let connect_change_task = tokio::spawn(async move {
            let proxy = IpcRulerProxy::builder(&shared_bus)
                .cache_properties(zbus::CacheProperties::No)
                .build()
                .await
                .unwrap();
            loop {
                let res = proxy.receive_connection_change().await;
                if res.is_ok() {
                    let args = res.unwrap().next().await.unwrap();
                    let slot_name = args.args().unwrap().slot_name().to_string();
                    if slot_name == shared_name {
                        shared_connect_notify.notify_waiters();
                        // let _ = shared_slot.write().await.disconnect().await;
                        let signal_name = args.args().unwrap().signal_name().to_string();
                        log!(target: &shared_log_key, Level::Trace,
                            "Connection change received for slot: {}, signal: {}", slot_name, signal_name);
                        let res = shared_slot.write().await.connect(&signal_name);
                        if res.is_err() {
                            log!(target: &shared_log_key, Level::Error,
                                "Failed to connect to signal: {}", res.err().unwrap());
                        }
                    }
                } else {
                }
            }
        });

        let shared_log_key = log_key.clone();
        let shared_name = base.full_name();
        let shared_description = base.description.clone().unwrap_or(String::new());
        let shared_bus = bus.clone();
        // register the dbus interface for this slot
        let register_task = tokio::spawn(async move {
            let proxy = IpcRulerProxy::builder(&shared_bus)
                .cache_properties(zbus::CacheProperties::No)
                .build()
                .await
                .unwrap();
            loop {
                let res = proxy
                    .register_slot(
                        shared_name.as_str(),
                        shared_description.as_str(),
                        T::type_identifier(),
                    )
                    .await;
                if res.is_ok() {
                    break;
                } else {
                    log!(target: &shared_log_key, Level::Trace,
                    "Slot Registration failed: '{}', will try again in 1s", res.err().unwrap());
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                }
            }
        });

        let shared_name = base.name.clone();
        let shared_log_key = log_key.clone();
        let shared_connect_notify = Arc::clone(&connect_notify);
        let shared_slot = Arc::clone(&slot);
        let shared_new_value_sender = new_value_sender.clone();
        let shared_bus = bus.clone();
        let recv_task: JoinHandle<()> = tokio::spawn(async move {
            let mut local_last_value: Option<T> = None;

            let filters = Filters::new(
                shared_bus.clone(),
                format!("filters/{}", Base::<T>::type_and_name(&shared_name)).as_str(),
            );
            loop {
                let slot_guard = shared_slot.read().await; // Create a binding for the read guard
                let new_value = tokio::select! {
                    result = slot_guard.recv() => {
                        match result {
                            Ok(value) => {
                                filters.process(value, &local_last_value).await
                            }
                            Err(e) => {
                                Err(e)
                            }
                        }
                    },
                    _ = shared_connect_notify.notified() => {
                        Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "Connecting to other signal, let's try receiving from it",
                        )) as Box<dyn std::error::Error + Send + Sync>)
                    }
                };

                match new_value {
                    Ok(filtered) => {
                        local_last_value = Some(filtered);
                        debug!(target: &shared_log_key, "Value sent: {:?}", local_last_value);
                        let _ = shared_new_value_sender
                            .send(local_last_value.clone())
                            .map_err(|e| {
                                log!(target: &shared_log_key, Level::Error,
                                "Error sending new value: {}", e);
                            });
                    }
                    Err(err) => {
                        debug!(target: &shared_log_key, "Value not sent: {}", err);
                    }
                }
            }
        });

        Self {
            slot,
            connect_notify,
            log_key,
            new_value_signal: new_value_sender,
            connect_change_task,
            register_task,
            recv_task,
            base,
        }
    }

    pub async fn async_recv(&self) -> Result<Option<T>, Box<dyn Error + Send + Sync>> {
        let mut watcher = self.subscribe();
        // todo return error if we can't watch
        let _ = watcher.changed().await.map_err(|_| {
            log!(target: &self.log_key, Level::Error,
            "recv Error watching for changes");
        });
        let value = watcher.borrow_and_update().clone();
        Ok(value)
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<T>> {
        self.new_value_signal.subscribe()
    }

    // return tuple of watch recv and send
    pub fn channel(&self, name: &str) -> (watch::Sender<Option<T>>, watch::Receiver<Option<T>>) {
        let (tx, mut rx) = watch::channel(None as Option<T>);
        let log_key = self.log_key.clone();
        let self_tx = self.new_value_signal.clone();
        let name = name.to_string();
        tokio::spawn(async move {
            loop {
                rx.changed().await.expect("Error watching for changes");
                let value = rx.borrow().clone();
                trace!(target: &log_key, "Changed {} value to {:?}", name, value); // OPC-UA Changed Temperature value to 10
                                                                                   // DBUS-Tinker Changed Temperature value to -1
                                                                                   // Override current slot value with tinkered value.
                self_tx.send(value).expect("Error sending value to slot");
            }
        });
        (tx, self.new_value_signal.subscribe())
    }

    pub fn recv(&mut self, callback: Box<dyn Fn(&T) + Send + Sync>)
    where
        <T as AnyFilterDecl>::Type: Send + Sync + Filter<T>,
    {
        let mut watcher = self.subscribe();
        let log_key = self.log_key.clone();
        tokio::spawn(async move {
            loop {
                let _ = watcher.changed().await.map_err(|_| {
                    log!(target: &log_key, Level::Error,
                    "recv Error watching for changes");
                });
                let value = watcher.borrow();
                callback(value.as_ref().unwrap());
            }
        });
    }

    pub async fn async_connect(
        &mut self,
        signal_name: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.connect_notify.notify_waiters();
        self.slot.write().await.async_connect(signal_name).await
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

    pub fn base(&self) -> Base<T> {
        Base::new(&self.base.name, self.base.description.as_deref())
    }
}
impl<T: Send + Sync + 'static + PartialEq + AnyFilterDecl> Drop for Slot<T>
where
    <T as AnyFilterDecl>::Type: Filter<T>,
{
    fn drop(&mut self) {
        self.register_task.abort();
        self.connect_change_task.abort();
        self.recv_task.abort();
    }
}

pub struct SlotImpl<T> {
    base: Base<T>,
    sock: Arc<Mutex<Option<SubSocket>>>,
    is_connected: Arc<Mutex<bool>>,
    connect_notify: Arc<Notify>,
    connect_task: Option<tokio::task::JoinHandle<()>>,
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
            sock: Arc::new(Mutex::new(None)),
            is_connected: Arc::new(Mutex::new(false)),
            connect_notify: Arc::new(Notify::new()),
            connect_task: None,
        }
    }
    async fn async_connect_(
        log_key: &str,
        sock: Arc<Mutex<Option<SubSocket>>>,
        signal_name: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if signal_name.is_empty() {
            return Ok(());
        }
        let socket_path = endpoint(signal_name);
        log!(target: &log_key, Level::Trace, "Trying to connect to: {}", socket_path);
        let mut sock = sock.lock();
        if sock.is_none() {
            *sock = Some(SubSocket::new());
        }
        sock.as_mut().unwrap().connect(socket_path.as_str()).await?;
        sock.as_mut().unwrap().subscribe("").await?;
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
            *self.is_connected.lock() = true;
            self.connect_notify.notify_waiters();
        }
        // little hack to make sure the connection is established
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        res
    }
    pub fn connect(&mut self, signal_name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let shared_sock = Arc::clone(&self.sock);
        let signal_name_str = signal_name.to_string();
        let log_key_cp = self.base.log_key.clone();
        let shared_is_connected = Arc::clone(&self.is_connected);
        let shared_connect_notify = Arc::clone(&self.connect_notify);

        // cancel any previous connect
        if let Some(task) = self.connect_task.take() {
            task.abort();
        }
        self.connect_task = Some(tokio::spawn(async move {
            {
                let mut socket = shared_sock.lock();
                if let Some(sub_socket) = socket.take() {
                    sub_socket.close().await;
                }
                *socket = None;
                if signal_name_str.is_empty() {
                    *shared_is_connected.lock() = false;
                    shared_connect_notify.notify_waiters();
                    return;
                }
            }
            loop {
                let connect_res = tokio::time::timeout(
                    tokio::time::Duration::from_millis(100),
                    Self::async_connect_(
                        log_key_cp.as_str(),
                        Arc::clone(&shared_sock),
                        signal_name_str.as_str(),
                    ),
                )
                .await;
                match connect_res {
                    Ok(Ok(())) => {
                        log!(target: &log_key_cp, Level::Trace,
                            "Connect to: {:?} successful", signal_name_str);
                        *shared_is_connected.lock() = true;
                        shared_connect_notify.notify_waiters();
                        break;
                    }
                    Ok(Err(e)) => {
                        log!(target: &log_key_cp, Level::Trace,
                            "Connect to: {:?} failed: {:?}, will try again", signal_name_str, e);
                    }
                    Err(_) => {
                        log!(target: &log_key_cp, Level::Trace,
                            "Connect to: {:?} timed out, will try again", signal_name_str);
                    }
                }
            }
        }));
        Ok(())
    }
    pub async fn recv(&self) -> Result<T, Box<dyn Error + Send + Sync>> {
        if !*self.is_connected.lock() {
            log!(target: self.base.log_key.as_str(), Level::Trace, "Still not connected will wait until then");
            self.connect_notify.notified().await;
        }
        // todo tokio select, if we were to disconnect in the middle of recv
        let mut sock = self.sock.lock();
        if sock.is_none() {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Socket is not connected",
            )));
        }
        let buffer: ZmqMessage = sock.as_mut().unwrap().recv().await?;
        // todo remove copying
        let flattened_buffer: Vec<u8> = buffer.iter().flat_map(|b| b.to_vec()).collect();
        let mut cursor = io::Cursor::new(flattened_buffer);
        let deserialized_packet =
            DeserializePacket::<T>::deserialize(&mut cursor).expect("Deserialization failed");
        Ok(deserialized_packet.value)
    }
}

pub struct Signal<T: TypeName> {
    base: Base<T>,
    value_sender: watch::Sender<Option<T>>,
    init_task: Option<tokio::task::JoinHandle<()>>,
    monitor_task: tokio::task::JoinHandle<()>,
    register_task: Option<tokio::task::JoinHandle<()>>,
}

impl<T> Signal<T>
where
    T: TypeName
        + TypeIdentifier
        + SerializeSize
        + Serialize
        + Deserialize
        + std::fmt::Debug
        + Sync
        + Send
        + 'static,
{
    pub fn new(base: Base<T>) -> Self {
        let (value_sender, mut value_receiver) = watch::channel(None as Option<T>);

        let (mut sock_sender, mut sock_receiver) = mpsc::channel::<PubSocket>(1);
        let path = base.path();
        let endpoint = base.endpoint();
        let log_key = base.log_key.clone();
        let init_task = tokio::spawn(async move {
            let mut sock = PubSocket::new();
            if path.exists() {
                std::fs::remove_file(path)
                    .expect(format!("Failed to remove file: {}", endpoint).as_str());
            }
            sock.bind(&endpoint)
                .await
                .map_err(|e| {
                    log!(target: &log_key, Level::Error, "Failed to bind to endpoint: {}", e);
                    e
                })
                .expect(format!("Failed to bind to endpoint: {}", endpoint).as_str());
            trace!(target: &log_key, "Bound to endpoint: {}", endpoint);
            sock_sender.send(sock).await.expect("Failed to send socket");
        });

        let log_key = base.log_key.clone();
        let monitor_task = tokio::spawn(async move {
            let mut sock: PubSocket = loop {
                if let Some(sock) = sock_receiver.next().await {
                    break sock;
                }
            };

            let mut monitor = sock.monitor();

            loop {
                select! {
                    // TODO let's use different zmq topic to propagate the last value
                    event = monitor.next() => {
                        match event {
                            Some(SocketEvent::Accepted { .. }) => {
                                let mut buffer: Vec<u8> = Vec::new();
                                {
                                    // this is scoped to prevent holding onto borrow over await point
                                    let value = value_receiver.borrow();
                                    trace!(target: &log_key, "Accepted event, last value: {:?}", value);
                                    if value.is_none() {
                                        continue;
                                    }
                                    let packet = SerializePacket::new(value.as_ref().unwrap());
                                    packet.serialize(&mut buffer).expect("Serialization failed");
                                }
                                // TODO use ZMQ_EVENT_HANDSHAKE_SUCCEEDED and throw this sleep out
                                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                                sock.send(ZmqMessage::from(buffer))
                                    .await
                                    .expect("Failed to send value");
                            }
                            other_event => {
                                log!(target: &log_key, Level::Info,
                                    "Other event: {:?}, last value: {:?}",
                                    other_event,
                                    value_receiver.borrow()
                                );
                            }
                        }
                    },
                    _ = value_receiver.changed() => {
                        let mut buffer: Vec<u8> = Vec::new();
                        {
                            // this is scoped to prevent holding onto borrow over await point
                            let value = value_receiver.borrow_and_update();
                            if value.is_none() {
                                continue;
                            }
                            let packet = SerializePacket::new(value.as_ref().unwrap());
                            packet.serialize(&mut buffer).expect("Serialization failed");
                        }
                        if !buffer.is_empty() {
                            sock.send(ZmqMessage::from(buffer))
                                .await
                            .expect("Failed to send value");
                        }
                    }
                };
            }
        });

        Self {
            base,
            value_sender,
            init_task: Some(init_task),
            monitor_task,
            register_task: None,
        }
    }

    pub fn register(mut self, bus: zbus::Connection) -> Self {
        let name = self.base.full_name();
        let description = self.base.description.clone().unwrap_or(String::new());
        let log_key = self.base.log_key.clone();
        self.register_task = Some(tokio::spawn(async move {
            let proxy = IpcRulerProxy::builder(&bus)
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
                    trace!(target: &log_key, "Signal Registration failed: '{}', will try again in 1s", res.err().unwrap());
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                }
            }
        }));
        self
    }

    pub async fn async_send(&mut self, value: T) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.value_sender.send(Some(value))?;
        tokio::task::yield_now().await;
        Ok(())
    }

    pub fn send(&mut self, value: T) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.value_sender.send(Some(value))?;
        Ok(())
    }

    pub fn base(&self) -> Base<T> {
        if let Some(ref description) = self.base.description {
            Base::new(&self.base.name, Some(description.as_str()))
        } else {
            Base::new(&self.base.name, None)
        }
    }

    // Receive changes to the signal
    pub fn subscribe(&self) -> watch::Receiver<Option<T>> {
        // todo I would like to prioritize the zmq send over this
        self.value_sender.subscribe()
    }

    pub async fn init_task(&mut self) -> Result<(), tokio::task::JoinError> {
        self.init_task.take().unwrap().await
    }
}

impl<T: TypeName> Drop for Signal<T> {
    fn drop(&mut self) {
        if let Some(init_task) = self.init_task.take() {
            init_task.abort();
        }
        self.monitor_task.abort();
        if let Some(register_task) = self.register_task.take() {
            register_task.abort();
        }
        let file = self.base.endpoint();
        std::fs::remove_file(file).unwrap_or(());
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

pub trait SerializeSize {
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
pub trait Serialize {
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
    #[allow(dead_code)]
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
