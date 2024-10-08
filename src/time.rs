use schemars::{
    gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject},
    JsonSchema,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::time::Duration;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum DurationType {
    Nano = 0,
    Micro = 1,
    Milli = 2,
    Sec = 3,
}
const NANO: u8 = DurationType::Nano as u8;
const MICRO: u8 = DurationType::Micro as u8;
const MILLI: u8 = DurationType::Milli as u8;
const SEC: u8 = DurationType::Sec as u8;

impl From<DurationType> for u8 {
    fn from(duration_type: DurationType) -> Self {
        duration_type as u8
    }
}

impl From<u8> for DurationType {
    fn from(duration_type: u8) -> Self {
        match duration_type {
            NANO => DurationType::Nano,
            MICRO => DurationType::Micro,
            MILLI => DurationType::Milli,
            SEC => DurationType::Sec,
            _ => panic!("Invalid duration type"),
        }
    }
}

impl ToString for DurationType {
    fn to_string(&self) -> String {
        match self {
            DurationType::Nano => "ns".to_string(),
            DurationType::Micro => "µs".to_string(),
            DurationType::Milli => "ms".to_string(),
            DurationType::Sec => "s".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct GenericDuration<const TYPE: u8>(Duration);

impl<const TYPE: u8> From<GenericDuration<TYPE>> for Duration {
    fn from(nano_duration: GenericDuration<TYPE>) -> Self {
        nano_duration.0
    }
}

impl<const TYPE: u8> From<Duration> for GenericDuration<TYPE> {
    fn from(duration: Duration) -> Self {
        GenericDuration(duration)
    }
}

impl<const TYPE: u8> PartialEq<Duration> for GenericDuration<TYPE> {
    fn eq(&self, other: &Duration) -> bool {
        self.0 == *other
    }
}

impl<const TYPE: u8> PartialEq<GenericDuration<TYPE>> for Duration {
    fn eq(&self, other: &GenericDuration<TYPE>) -> bool {
        *self == other.0
    }
}

impl<const TYPE: u8> PartialOrd<Duration> for GenericDuration<TYPE> {
    fn partial_cmp(&self, other: &Duration) -> Option<Ordering> {
        self.0.partial_cmp(other)
    }
}

impl<const TYPE: u8> PartialOrd<GenericDuration<TYPE>> for Duration {
    fn partial_cmp(&self, other: &GenericDuration<TYPE>) -> Option<Ordering> {
        self.partial_cmp(&other.0)
    }
}

impl<const TYPE: u8> Serialize for GenericDuration<TYPE> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let value = match TYPE {
            NANO => self.0.as_nanos(),
            MICRO => self.0.as_micros(),
            MILLI => self.0.as_millis(),
            SEC => self.0.as_secs() as u128,
            _ => panic!("Invalid duration type"),
        };
        serializer.serialize_u128(value)
    }
}

impl<'de, const TYPE: u8> Deserialize<'de> for GenericDuration<TYPE> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u128::deserialize(deserializer)?;
        match TYPE {
            NANO => Ok(GenericDuration(Duration::from_nanos(value as u64))),
            MICRO => Ok(GenericDuration(Duration::from_micros(value as u64))),
            MILLI => Ok(GenericDuration(Duration::from_millis(value as u64))),
            SEC => Ok(GenericDuration(Duration::from_secs(value as u64))),
            _ => panic!("Invalid duration type"),
        }
    }
}

impl<const TYPE: u8> JsonSchema for GenericDuration<TYPE> {
    fn schema_name() -> String {
        format!("GenericDuration<{}>", DurationType::from(TYPE).to_string())
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let mut schema_obj = SchemaObject::default();
        schema_obj.instance_type = Some(InstanceType::Integer.into());
        schema_obj.format = Some("uint64".to_string());
        schema_obj.number().minimum = Some(0.0);
        schema_obj.number().maximum = Some(u64::MAX as f64);
        let unit = DurationType::from(TYPE).to_string();
        schema_obj
            .extensions
            .insert("unit".to_string(), serde_json::Value::String(unit));
        Schema::Object(schema_obj)
    }
}

pub type NanoDuration = GenericDuration<NANO>;
pub type MicroDuration = GenericDuration<MICRO>;
pub type MilliDuration = GenericDuration<MILLI>;
pub type SecDuration = GenericDuration<SEC>;
