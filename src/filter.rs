use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::cmp::PartialEq;
use std::error::Error;
use std::marker::{Send, Sync};
use std::ops::Add;
use std::ops::Not;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::confman::ConfMan;

#[async_trait]
pub trait Filter<T> {
    async fn filter(
        &self,
        new_value: T,
        old_value: Option<&T>,
    ) -> Result<T, Box<dyn Error + Send + Sync>>;
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct FilterNewState {}

#[async_trait]
impl<T: Send + Sync + 'static + PartialEq> Filter<T> for FilterNewState {
    async fn filter(
        &self,
        value: T,
        old_value: Option<&T>,
    ) -> Result<T, Box<dyn Error + Send + Sync>> {
        if old_value.is_none() {
            return Ok(value);
        }
        if old_value.unwrap() != &value {
            return Ok(value);
        }
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Same as last value",
        )))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct FilterInvert {}

#[async_trait]
impl<T: Send + Sync + 'static + PartialEq + Not<Output = T>> Filter<T> for FilterInvert {
    async fn filter(&self, value: T, _: Option<&T>) -> Result<T, Box<dyn Error + Send + Sync>> {
        Ok(!value)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct FilterTimer {
    time_on: Duration,
    time_off: Duration,
    #[serde(skip)]
    state: Arc<Mutex<Option<CancellationToken>>>,
}

#[async_trait]
impl Filter<bool> for FilterTimer {
    async fn filter(
        &self,
        value: bool,
        _: Option<&bool>,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        // If value is true, wait time_on; otherwise, wait time_off.
        // If a new value arrives before the timer expires, cancel the current timer.

        let mut state = self.state.lock().await;

        if let Some(ref token) = *state {
            token.cancel();
        }

        let new_token = CancellationToken::new();
        *state = Some(new_token.clone());

        let duration = if value { self.time_on } else { self.time_off };

        // Release the lock before asynchronous operations.
        drop(state);

        if duration == Duration::from_secs(0) {
            return Ok(value);
        }

        tokio::select! {
            _ = tokio::time::sleep(duration) => {
                // Check if the token has been cancelled.
                if !new_token.is_cancelled() {
                    return Ok(value);
                } else {
                    // The timer was cancelled before completion.
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Timer was cancelled by a new value",
                    )));
                }
            }
            _ = new_token.cancelled() => {
                // The timer was cancelled.
                return Err("Timer was cancelled by a new value".into());
            }
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct FilterOffset<T> {
    offset: T,
}

#[async_trait]
impl<T: Send + Sync + 'static + PartialEq + Add<T, Output = T> + Clone> Filter<T>
    for FilterOffset<T>
{
    async fn filter(&self, value: T, _: Option<&T>) -> Result<T, Box<dyn Error + Send + Sync>> {
        Ok(value + self.offset.clone())
    }
}

pub trait AnyFilterDecl {
    type ValueT;
    type Type: Filter<Self::ValueT>
        + Default
        + Serialize
        + for<'de> Deserialize<'de>
        + JsonSchema
        + Send
        + Sync
        + 'static;

    fn default_filters() -> Vec<Self::Type>;
}

macro_rules! impl_any_filter_decl {
    ($type:ty, $enum_name:ident, $default_filters:expr, { $($variant_name:ident($filter_type:ty)),* $(,)? }) => {
        impl AnyFilterDecl for $type {
            type ValueT = $type;
            type Type = $enum_name;

            fn default_filters() -> Vec<Self::Type> {
                $default_filters
            }
        }

        #[derive(Serialize, Deserialize, JsonSchema)]
        pub enum $enum_name {
            $($variant_name($filter_type)),*
        }
        impl Default for $enum_name {
            fn default() -> Self {
                $enum_name::NewState(FilterNewState {})
            }
        }

        #[async_trait]
        impl Filter<$type> for $enum_name {
            async fn filter(
                &self,
                new_value: $type,
                old_value: Option<&$type>,
            ) -> Result<$type, Box<dyn Error + Send + Sync>> {
                match self {
                    $(
                        $enum_name::$variant_name(filter) => {
                            filter.filter(new_value, old_value).await
                        }
                    ),*
                }
            }
        }
    };
}

impl_any_filter_decl!(
    bool,
    AnyFilterBool,
    vec![AnyFilterBool::NewState(FilterNewState {})],
    {
        NewState(FilterNewState),
        Invert(FilterInvert),
        Timer(FilterTimer)
        // Add other filters specific to bool
    }
);

impl_any_filter_decl!(
    i64,
    AnyFilterI64,
    vec![AnyFilterI64::NewState(FilterNewState {})],
    {
        NewState(FilterNewState),
        Offset(FilterOffset<i64>)
        // Add other filters specific to i64
    }
);

pub struct Filters<T>
where
    T: AnyFilterDecl + Send + Sync + 'static + PartialEq,
    <T as AnyFilterDecl>::Type: Filter<T>
        + Serialize
        + for<'de> Deserialize<'de>
        + JsonSchema
        + Default
        + Send
        + Sync
        + 'static,
{
    filters: ConfMan<Vec<<T as AnyFilterDecl>::Type>>,
    last_value: Arc<Mutex<Option<T>>>,
}

impl<T> Filters<T>
where
    T: AnyFilterDecl + Send + Sync + 'static + PartialEq,
    <T as AnyFilterDecl>::Type: Filter<T>
        + Serialize
        + for<'de> Deserialize<'de>
        + JsonSchema
        + Default
        + Send
        + Sync
        + 'static,
{
    pub fn new(bus: zbus::Connection, key: &str, last_value: Arc<Mutex<Option<T>>>) -> Self
    where
        <T as AnyFilterDecl>::Type:
            Send + Sync + Serialize + for<'de> Deserialize<'de> + JsonSchema,
    {
        Filters {
            filters: ConfMan::new(bus, key).with_default(<T as AnyFilterDecl>::default_filters()),
            last_value,
        }
    }
    pub async fn process(&self, new_value: T) -> Result<T, Box<dyn Error + Send + Sync>> {
        let old_value = self.last_value.lock().await;
        let old_value = old_value.as_ref();
        let mut result: Result<T, Box<dyn Error + Send + Sync>> = Ok(new_value);
        for filter in self.filters.read().iter() {
            if result.is_err() {
                break;
            }
            result = filter.filter(result?, old_value).await;
        }
        result
    }
}
