use futures::future::Either;
use futures::stream::StreamExt;
use futures_channel::mpsc;
use log::{log, Level};
use quantities::mass::Mass;
use std::io::Cursor;
use std::marker::PhantomData;
use std::{error::Error, io, path::PathBuf, sync::Arc, sync::RwLock};
use tokio::select;
use tokio::sync::{Mutex, Notify};
use zeromq::{
    PubSocket, Socket, SocketEvent, SocketOptions, SocketRecv, SocketSend, SubSocket, ZmqError,
    ZmqMessage,
};

use crate::progbase;

const FILE_PREFIX: &'static str = "ipc://";
const FILE_PATH: &'static str = "/var/run/tfc/";
fn endpoint(file_name: &str) -> String {
    format!("{}{}{}", FILE_PREFIX, FILE_PATH, file_name)
}

pub struct Base<T> {
    pub name: String,
    pub description: Option<String>,
    pub value: Arc<RwLock<Option<T>>>,
}

impl<T: TypeName> Base<T> {
    pub fn new(name: &str, description: Option<String>) -> Self {
        Self {
            name: String::from(name),
            description,
            value: Arc::new(RwLock::new(None)),
        }
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

pub struct Slot<T> {
    base: Base<T>,
    // callback: Box<dyn Fn(T)>,
    sock: Arc<Mutex<SubSocket>>,
    is_connected: bool,
    connect_notify: Arc<Notify>,
}

impl<T> Slot<T>
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
            is_connected: false,
            connect_notify: Arc::new(Notify::new()),
        }
    }
    async fn async_connect_(
        sock: Arc<Mutex<SubSocket>>,
        signal_name: &str,
    ) -> Result<(), Box<dyn Error>> {
        let socket_path = endpoint(signal_name);
        println!("Trying to connect to: {}", socket_path);
        sock.lock().await.connect(socket_path.as_str()).await?;
        sock.lock().await.subscribe("").await?;
        Ok(())
    }
    pub async fn async_connect(&mut self, signal_name: &str) -> Result<(), Box<dyn Error>> {
        // todo reconnect
        Self::async_connect_(Arc::clone(&self.sock), signal_name).await
    }
    pub fn connect(&mut self, signal_name: &str) -> Result<(), Box<dyn Error>> {
        let shared_sock = Arc::clone(&self.sock);
        let shared_connect_notify = Arc::clone(&self.connect_notify);
        let signal_name_str = signal_name.to_string();
        tokio::spawn(async move {
            loop {
                let connect_task = async {
                    Self::async_connect_(Arc::clone(&shared_sock), signal_name_str.as_str()).await
                };
                let timeout_task =
                    async { tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await };
                match select! {
                    result = connect_task => Either::Left(result),
                    _ = timeout_task => Either::Right(()),
                } {
                    Either::Left(Ok(_)) => {
                        println!("Connection successful");
                        shared_connect_notify.notify_waiters();
                        break;
                    }
                    Either::Left(Err(e)) => {
                        eprintln!("Connection failed: {}", e);
                    }
                    Either::Right(_) => {
                        eprintln!("Connection timed out: {}", signal_name_str);
                    }
                }
            }
        });
        Ok(())
    }
    pub async fn recv(&mut self) -> Result<T, Box<dyn Error>> {
        if !self.is_connected {
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
    base: Base<T>,
    sock: Arc<Mutex<PubSocket>>,
    monitor: Option<mpsc::Receiver<SocketEvent>>,
}

impl<T> Signal<T>
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
        let cp_name = self.base.full_name();
        if let Some(mut receiver) = self.monitor.take() {
            tokio::spawn(async move {
                while let Some(event) = receiver.next().await {
                    match event {
                        SocketEvent::Accepted { .. } => {
                            log!(target: &cp_name, Level::Trace,
                                "Accepted event, last value: {:?}", shared_value.read().unwrap()
                            );
                            let locked_sock = shared_sock.lock();
                            let mut buffer = Vec::new();
                            {
                                let value = shared_value.read().unwrap();
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
                            log!(target: &cp_name, Level::Info,
                                "Other event: {:?}, last value: {:?}",
                                other_event,
                                shared_value.read().unwrap()
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
        *self.base.value.write().unwrap() = Some(value);
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
