use std::{
    error::Error,
    fmt::{Debug, Display},
};
use thiserror::Error;

/// Result of a call into the scan thread.
pub type ScanThreadResult<T = ()> = Result<T, ScanThreadError>;

/// The payload a scan thread panicked with, kept so it can be shown as an error message.
#[derive(Debug)]
pub struct ThreadJoinError(Box<dyn std::any::Any + Send + 'static>);

/// Why talking to the scan thread failed.
#[derive(Debug, Error)]
pub enum ScanThreadError {
    #[error("error at thread join: {0}")]
    ThreadJoin(#[from] ThreadJoinError),
    #[error("error while trying to recive value from scan thread: {0}")]
    ThreadRecieve(#[from] std::sync::mpsc::TryRecvError),
}

impl ThreadJoinError {
    /// Wraps the payload returned by `JoinHandle::join` for a panicked thread.
    pub fn new(v: Box<dyn std::any::Any + Send + 'static>) -> Self {
        Self(v)
    }
}
impl Error for ThreadJoinError {}
/// Shows the panic message when the payload is a `&str` or `String` (what `panic!` produces),
/// and a placeholder for anything else.
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc::TryRecvError;

    use super::*;

    #[test]
    fn a_str_panic_payload_is_shown_as_is() {
        let error = ThreadJoinError::new(Box::new("boom"));
        assert_eq!(error.to_string(), "boom");
    }

    #[test]
    fn a_string_panic_payload_is_shown_as_is() {
        let error = ThreadJoinError::new(Box::new(String::from("formatted boom 42")));
        assert_eq!(error.to_string(), "formatted boom 42");
    }

    #[test]
    fn any_other_payload_gets_a_placeholder() {
        let error = ThreadJoinError::new(Box::new(42_i32));
        assert_eq!(error.to_string(), "Any { .. }");
    }

    #[test]
    fn join_errors_carry_the_panic_message() {
        let error = ScanThreadError::from(ThreadJoinError::new(Box::new("boom")));
        assert_eq!(error.to_string(), "error at thread join: boom");
    }

    #[test]
    fn a_closed_channel_is_reported_as_a_receive_error() {
        let error = ScanThreadError::from(TryRecvError::Disconnected);
        assert!(error.to_string().starts_with("error while trying to recive value"));
    }
}
