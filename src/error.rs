use std::{
    error::Error,
    fmt::{Debug, Display},
};
use thiserror::Error;

pub type MainResult<T = ()> = Result<T, MainError>;
pub type ScanThreadResult<T = ()> = Result<T, ScanThreadError>;

#[derive(Debug, Error)]
pub enum MainError {
    #[error("{0}")]
    Scan(#[from] ScanThreadError),
}

#[derive(Debug)]
pub struct ThreadJoinError(Box<dyn std::any::Any + Send + 'static>);

#[derive(Debug, Error)]
pub enum ScanThreadError {
    #[error("error at thread join: {0}")]
    ThreadJoin(#[from] ThreadJoinError),
    #[error("error while trying to recive value from scan thread: {0}")]
    ThreadRecieve(#[from] std::sync::mpsc::TryRecvError),
}

impl ThreadJoinError {
    pub fn new(v: Box<dyn std::any::Any + Send + 'static>) -> Self {
        Self(v)
    }
}
impl Error for ThreadJoinError {}
impl Display for ThreadJoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(message) = self.0.downcast_ref::<&str>() {
            Display::fmt(&message, f)
        } else if let Some(message) = self.0.downcast_ref::<String>() {
            Display::fmt(&message, f)
        } else {
            self.0.fmt(f)
        }
    }
}
