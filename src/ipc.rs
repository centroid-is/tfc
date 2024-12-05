use futures::stream::StreamExt;
use futures::SinkExt;
use futures_channel::mpsc;
use log::{debug, error, log, trace, warn, Level};
use quantities::mass::Mass;
use std::marker::PhantomData;
use std::{error::Error, io, path::PathBuf};
use tokio::select;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use zeromq::{PubSocket, Socket, SocketEvent, SocketRecv, SocketSend, SubSocket, ZmqMessage};

use crate::filter::AnyFilterDecl;
use crate::filter::Filter;
use crate::filter::Filters;
use crate::ipc_ruler_client::IpcRulerProxy;
use crate::progbase;

pub mod dbus;
pub mod opcua;

const ZMQ_PREFIX: &'static str = "ipc://";
fn path() -> PathBuf {
    const FILE_PATH: &'static str = "/var/run/tfc/";
    PathBuf::from(std::env::var("RUNTIME_DIRECTORY").unwrap_or_else(|_| FILE_PATH.to_string()))
}
fn endpoint(file_name: &str) -> String {
    let path = path().join(file_name);
    format!("{}{}", ZMQ_PREFIX, path.display())
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
        path().join(self.full_name())
    }
}

pub struct Slot<T> {
    base: Base<T>,
    sock_sender: mpsc::Sender<SubSocket>,
    new_value_sender: watch::Sender<Option<T>>,
    recv_task: Option<JoinHandle<()>>,
    register_task: Option<JoinHandle<()>>,
    connection_change_task: Option<JoinHandle<()>>,
}

impl<T> Slot<T>
where
    T: TypeName
        + TypeIdentifier
        + Deserialize
        + Send
        + Sync
        + 'static
        + PartialEq
        + Clone
        + AnyFilterDecl
        + std::fmt::Debug,
    <T as AnyFilterDecl>::Type: Filter<T>,
{
    pub fn new(dbus: zbus::Connection, base: Base<T>) -> Self {
        let (sock_sender, sock_receiver) = mpsc::channel::<SubSocket>(1);
        let (new_value_sender, _) = watch::channel(None as Option<T>);

        let this = Self {
            base,
            sock_sender,
            new_value_sender,
            recv_task: None,
            register_task: None,
            connection_change_task: None,
        };
        let mut this = this.register(dbus.clone());
        this.start_recv_task(sock_receiver, dbus);
        this
    }

    fn start_recv_task(
        &mut self,
        mut sock_receiver: mpsc::Receiver<SubSocket>,
        bus: zbus::Connection,
    ) {
        let new_value_sender = self.new_value_sender.clone();
        let log_key = self.base.log_key.clone();
        let short_name = self.base.name.clone();
        self.recv_task = Some(tokio::spawn(async move {
            let mut sock: Option<SubSocket> = None;
            let mut local_last_value: Option<T> = None;
            let filters = Filters::new(
                bus.clone(),
                format!("filters/{}", Base::<T>::type_and_name(&short_name)).as_str(),
            );
            loop {
                select! {
                    // todo do we need to forward declare the stream here?
                    new_sock = sock_receiver.next() => {
                        sock = new_sock;
                    },
                    result = async {
                        if let Some(ref mut s) = sock {
                            s.recv().await
                        } else {
                            futures::future::pending().await
                        }
                    } => {
                        match result {
                            Ok(msg) => {
                                // todo remove copying
                                let flattened_buffer: Vec<u8> = msg.iter().flat_map(|b| b.to_vec()).collect();
                                let mut cursor = io::Cursor::new(flattened_buffer);
                                let deserialized_packet =
                                    DeserializePacket::<T>::deserialize(&mut cursor).expect("Deserialization failed");
                                match filters.process(deserialized_packet.value, &local_last_value).await {
                                    Ok(filtered) => {
                                        local_last_value = Some(filtered);
                                        // todo can we do this without cloning?
                                        new_value_sender.send(local_last_value.clone()).expect("Failed to send new value");
                                    }
                                    Err(e) => {
                                        debug!(target: &log_key, "Error processing/filtering value: {}", e);
                                    }
                                }
                            },
                            Err(e) => {
                                error!(target: &log_key, "Error receiving message: {}", e);
                            }
                        }
                    }
                }
            }
        }));
    }

    pub fn register(mut self, bus: zbus::Connection) -> Self {
        let description = self.base.description.clone().unwrap_or(String::new());
        let log_key = self.base.log_key.clone();
        let sock_sender = self.sock_sender.clone();
        let bus_cp = bus.clone();
        let full_name = self.base.full_name();
        self.register_task = Some(tokio::spawn(async move {
            let proxy = IpcRulerProxy::builder(&bus_cp)
                .cache_properties(zbus::CacheProperties::No)
                .build()
                .await
                .unwrap();
            loop {
                let res = proxy
                    .register_slot(
                        full_name.as_str(),
                        description.as_str(),
                        T::type_identifier(),
                    )
                    .await;
                if res.is_ok() {
                    break;
                } else {
                    trace!(target: &log_key, "Slot Registration failed: '{}', will try again in 1s", res.err().unwrap());
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                }
            }
        }));

        let my_name = self.base.full_name();
        let log_key = self.base.log_key.clone();
        self.connection_change_task = Some(tokio::spawn(async move {
            let proxy = IpcRulerProxy::builder(&bus)
                .cache_properties(zbus::CacheProperties::No)
                .build()
                .await
                .unwrap();
            loop {
                let res = proxy.receive_connection_change().await;
                if res.is_ok() {
                    let args = res.unwrap().next().await.unwrap();
                    let slot_name = args.args().unwrap().slot_name().to_string();
                    if slot_name == my_name {
                        let signal_name = args.args().unwrap().signal_name().to_string();
                        let _ = Self::async_connect_(&log_key, sock_sender.clone(), &signal_name)
                            .await
                            .map_err(|e| {
                                log!(target: &log_key, Level::Error,
                                "Failed to connect to signal: {} error: {}", signal_name, e);
                            });
                    }
                } else {
                }
            }
        }));

        self
    }

    async fn async_connect_(
        log_key: &str,
        mut sock_sender: mpsc::Sender<SubSocket>,
        signal_name: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if signal_name.is_empty() {
            return Err("Signal name is empty".into());
        }
        let mut sock = SubSocket::new();
        let socket_path = endpoint(signal_name);
        debug!(target: &log_key, "Trying to connect to: {}", socket_path);
        loop {
            use tokio::time::{sleep, timeout, Duration};

            match timeout(Duration::from_millis(100), sock.connect(&socket_path)).await {
                Ok(inner_result) => match inner_result {
                    Ok(_) => {
                        break;
                    }
                    Err(e) => {
                        debug!(target: &log_key, "Failed to connect to: {}, err: {}", socket_path, e);
                        sleep(Duration::from_millis(100)).await;
                    }
                },
                Err(_) => {
                    debug!(target: &log_key, "Timeout connecting to: {}", socket_path);
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
        debug!(target: &log_key, "Connected to: {}", socket_path);
        sock.subscribe("").await?;
        // little hack to make sure the connection is established
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        sock_sender.send(sock).await?;
        Ok(())
    }
    pub async fn async_connect(
        &mut self,
        signal_name: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Self::async_connect_(&self.base.log_key, self.sock_sender.clone(), signal_name).await
    }

    pub fn channel(&self, name: &str) -> (watch::Sender<Option<T>>, watch::Receiver<Option<T>>) {
        let (tx, mut rx) = watch::channel(None as Option<T>);
        let log_key = self.base.log_key.clone();
        let self_tx = self.new_value_sender.clone();
        let name = name.to_string();
        tokio::spawn(async move {
            loop {
                match rx.changed().await {
                    Ok(_) => {
                        let value = rx.borrow_and_update().clone();
                        trace!(target: &log_key, "Changed {} value to {:?}", name, value); // OPC-UA Changed Temperature value to 10
                                                                                           // DBUS-Tinker Changed Temperature value to -1
                                                                                           // Override current slot value with tinkered value.
                        self_tx.send(value).expect("Error sending value to slot");
                    }
                    Err(_) => {
                        warn!(target: &log_key, "Channel name dropped: {}", name);
                        break;
                    }
                }
            }
        });
        (tx, self.new_value_sender.subscribe())
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<T>> {
        self.new_value_sender.subscribe()
    }

    pub async fn async_recv(&self) -> Result<Option<T>, Box<dyn Error + Send + Sync>> {
        let mut watcher = self.subscribe();
        watcher.changed().await.map_err(|e| {
            log!(target: &self.base.log_key, Level::Error,
            "recv Error watching for changes: {}", e);
            e
        })?;
        let value = watcher.borrow_and_update().clone();
        Ok(value)
    }

    pub fn recv(&mut self, callback: Box<dyn Fn(&T) + Send + Sync>) {
        let mut watcher = self.subscribe();
        let log_key = self.base.log_key.clone();
        tokio::spawn(async move {
            loop {
                match watcher.changed().await {
                    Ok(_) => {
                        let value = watcher.borrow_and_update();
                        callback(value.as_ref().unwrap());
                    }
                    Err(e) => {
                        warn!(target: &log_key, "recv Error watching for changes: {}. Closing recv loop.", e);
                        break;
                    }
                }
            }
        });
    }

    pub fn base(&self) -> Base<T> {
        if let Some(ref description) = self.base.description {
            Base::new(&self.base.name, Some(description.as_str()))
        } else {
            Base::new(&self.base.name, None)
        }
    }
}

impl<T> Drop for Slot<T> {
    fn drop(&mut self) {
        if let Some(recv_task) = self.recv_task.take() {
            recv_task.abort();
        }
        if let Some(register_task) = self.register_task.take() {
            register_task.abort();
        }
        if let Some(connection_change_task) = self.connection_change_task.take() {
            connection_change_task.abort();
        }
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
        + Clone
        + Sync
        + Send
        + 'static,
{
    pub fn new(dbus: zbus::Connection, base: Base<T>) -> Self {
        let this = Self::new_raw(base);
        this.register(dbus)
    }

    pub fn new_raw(base: Base<T>) -> Self {
        let (value_sender, mut value_receiver) = watch::channel(None as Option<T>);

        let (mut sock_sender, mut sock_receiver) = mpsc::channel::<PubSocket>(1);
        let path = base.path();
        let endpoint = base.endpoint();
        let log_key = base.log_key.clone();
        let init_task = tokio::spawn(async move {
            let mut sock = PubSocket::new();
            if let Some(parent) = path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent).expect(
                        format!("Failed to create directory: {}", parent.display()).as_str(),
                    );
                }
            }
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

    pub async fn async_send(&self, value: T) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.value_sender.send(Some(value))?;
        tokio::task::yield_now().await;
        Ok(())
    }

    pub fn send(&self, value: T) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.value_sender.send(Some(value))?;
        Ok(())
    }

    // Receive changes to the signal
    pub fn subscribe(&self) -> watch::Receiver<Option<T>> {
        // todo I would like to prioritize the zmq send over this
        self.value_sender.subscribe()
    }

    // Offer a channel to send changes to the signal
    pub fn channel(&self, name: &str) -> (watch::Sender<Option<T>>, watch::Receiver<Option<T>>) {
        let (tx, mut rx) = watch::channel(None as Option<T>);
        let log_key = self.base.log_key.clone();
        let self_tx = self.value_sender.clone();
        let name = name.to_string();
        tokio::spawn(async move {
            loop {
                match rx.changed().await {
                    Ok(_) => {
                        let value = rx.borrow_and_update().clone();
                        trace!(target: &log_key, "Changed {} value to {:?}", name, value); // OPC-UA Changed Temperature value to 10
                                                                                           // DBUS-Tinker Changed Temperature value to -1
                                                                                           // Override current slot value with tinkered value.
                        self_tx.send(value).expect("Error sending value to slot");
                    }
                    Err(_) => {
                        warn!(target: &log_key, "Channel name dropped: {}", name);
                        break;
                    }
                }
            }
        });
        (tx, self.value_sender.subscribe())
    }

    pub async fn init_task(&mut self) -> Result<(), tokio::task::JoinError> {
        self.init_task.take().unwrap().await
    }

    pub fn base(&self) -> Base<T> {
        if let Some(ref description) = self.base.description {
            Base::new(&self.base.name, Some(description.as_str()))
        } else {
            Base::new(&self.base.name, None)
        }
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
