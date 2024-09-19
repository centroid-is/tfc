use async_trait::async_trait;
use std::cmp::PartialEq;
use std::error::Error;
use std::marker::{Send, Sync};
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait Filter<T> {
    async fn filter(&self, value: T) -> Result<T, Box<dyn Error + Send + Sync>>;
}

pub struct NewState<T> {
    last_value: Arc<Mutex<Option<T>>>,
}

#[async_trait]
impl<T: Send + Sync + 'static + PartialEq> Filter<T> for NewState<T> {
    async fn filter(&self, value: T) -> Result<T, Box<dyn Error + Send + Sync>> {
        let last_value = self.last_value.lock().await;
        if last_value.is_none() {
            return Ok(value);
        }
        if last_value.as_ref().unwrap() != &value {
            return Ok(value);
        }
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Same as last value",
        )))
    }
}
