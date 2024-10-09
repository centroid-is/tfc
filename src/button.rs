use core::fmt;
use tokio::sync::Mutex as TMutex;
use tokio::time::timeout;
use std::{sync::{Mutex, Arc}, pin::Pin, time::Duration};
use futures::{Stream, task::{Poll, Context}};

use crate::ipc::Slot;

// Single button events based on time and frequency
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum ButtonEvent {
    Tap,
    DoubleTap,
    LongTap
}

impl fmt::Display for ButtonEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ButtonEvent::Tap => write!(f, "Tap"),
            ButtonEvent::DoubleTap => write!(f, "DoubleTap"),
            ButtonEvent::LongTap => write!(f, "LongTap"),
        }
    }
}


pub struct Button {
    slot: Arc<TMutex<Slot<bool>>>,
    ready: Arc<Mutex<Option<ButtonEvent>>>,
}

impl Button {
    pub fn new(slot: Slot<bool>) -> Self {
        Button { slot: Arc::new(TMutex::new(slot)), ready: Arc::new(Mutex::new(None)) }
    }
}

impl Stream for Button {
    type Item = ButtonEvent;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let state = (*self.ready.lock().unwrap()).clone();
        match state {
            Some(value) => {
                let mut r = self.ready.lock().unwrap();
                *r = None;
                Poll::Ready(Some(value))
            }
            None => {
                let waker = cx.waker().clone();
                let ready = self.ready.clone();
                let slot = self.slot.clone();
                tokio::spawn(async move {
                    let s = slot.lock().await;
                    // First event in sequence, wait until that value is true to start the sequence
                    let mut value = false;
                    while value == false {
                        value = s.async_recv().await.expect("Got some value").lock().unwrap().clone();
                    }
                    // Now that someone has actuated the button there are three choices
                    // 1. The button is held long enough to become a long press
                    // 2. The button is let go soon enough that there and not pressed again. Producing a tap.
                    // 3. The button is let go and retapped soon enough that there is a double tap

                    let long_press_lower = Duration::from_millis(1000);
                    let double_press_bounds = Duration::from_millis(400);

                    let value = timeout(long_press_lower, s.async_recv()).await;
                    match value {
                        Ok(_) => {
                            // Here we can either produce a tap or a double tap.
                            let res = timeout(double_press_bounds, s.async_recv()).await;
                            match res {
                                Ok(_) => {
                                    // We have another true event within bounds. This is a double tap
                                    // Note. There will be a following false event, I don't want to wait for it I think.
                                    let mut r = ready.lock().unwrap();
                                    *r = Some(ButtonEvent::DoubleTap);
                                    waker.wake();
                                },
                                Err(_) => {
                                    // No other event occured after our inital true-false sequence. This was a tap
                                    let mut r = ready.lock().unwrap();
                                    *r = Some(ButtonEvent::Tap);
                                    waker.wake();
                                },
                            }
                        }
                        Err(_) => {
                            // This was a long press
                            let mut r = ready.lock().unwrap();
                            *r = Some(ButtonEvent::LongTap);
                            waker.wake();
                        }
                    }
                });
                Poll::Pending
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None) // We don't know the size as this can be infinite
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn initialize_button(){
        todo!("Test");
    }
}