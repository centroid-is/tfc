use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::cmp::PartialEq;
use std::error::Error;
use std::marker::{Send, Sync};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::confman::ConfMan;

#[async_trait]
pub trait Filter<T> {
    async fn filter(
        &self,
        new_value: T,
        old_value: Option<&T>,
    ) -> Result<T, Box<dyn Error + Send + Sync>>;
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct NewState {}

#[async_trait]
impl<T: Send + Sync + 'static + PartialEq> Filter<T> for NewState {
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

#[derive(Serialize, Deserialize, JsonSchema)]
enum FilterVariant {
    NewState(NewState),
}

pub struct Filters<T> {
    filters: ConfMan<Vec<FilterVariant>>,
    last_value: Arc<Mutex<Option<T>>>,
}

impl<T> Filters<T>
where
    T: Send + Sync + 'static + PartialEq,
{
    pub fn new(bus: zbus::Connection, key: &str, last_value: Arc<Mutex<Option<T>>>) -> Self {
        Filters {
            filters: ConfMan::new(bus, key)
                .with_default(vec![FilterVariant::NewState(NewState {})]),
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
            match filter {
                FilterVariant::NewState(filter) => {
                    result = filter.filter(result.unwrap(), old_value).await;
                }
            }
        }
        result
    }
}
