use quantities::mass::Mass;
use std::io;
use std::marker::PhantomData;
use zeromq::{PubSocket, SocketOptions, SubSocket};

use crate::progbase;

const FILE_PREFIX: &'static str = "ipc://";
const FILE_PATH: &'static str = "/var/run/tfc/";

struct Base<T> {
    name: String,
    description: String,
    value: T,
}

impl<T: TypeName> Base<T> {
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
        format!("{}{}{}", FILE_PREFIX, FILE_PATH, self.full_name())
    }
}

pub struct Slot<T> {
    base: Base<T>,
    callback: Box<dyn Fn(T)>,
    sock: SubSocket,
}

impl<T> Slot<T> {
    pub fn new() {
        // todo: https://github.com/zeromq/zmq.rs/issues/196
        // let mut options = SocketOptions::default();
        // options.ZMQ_RECONNECT_IVL

        //
    }
}

pub struct Signal<T> {
    base: Base<T>,
    value: T,
    sock: PubSocket,
}

// ------------------ trait TypeName ------------------

trait TypeName {
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

trait TypeIdentifier {
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
    Mass => 7
}

// ------------------ enum Protocol Version ------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    Unknown,
    V0,
}

trait IsFundamental {}
impl IsFundamental for bool {}
impl IsFundamental for i64 {}
impl IsFundamental for i32 {}
impl IsFundamental for i16 {}
impl IsFundamental for i8 {}
impl IsFundamental for u64 {}
impl IsFundamental for u32 {}
impl IsFundamental for u16 {}
impl IsFundamental for u8 {}
impl IsFundamental for f64 {}
impl IsFundamental for f32 {}
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
        + std::mem::size_of::<quantities::AmountT>() // todo global quantities::AmountT is not great, should be T::AmountT
        + std::mem::size_of::<E>()
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

#[derive(Debug, Clone)]
struct Packet<V> {
    header: Header<V>,
    value: V,
}

impl<V> Packet<V>
where
    V: TypeIdentifier + Copy + Clone,
{
    // fn new(value: V) -> Self {
    //     // let header = Header::new()
    //     Self{}
    // }
}
