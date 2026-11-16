use std::{
    sync::{
        self,
        mpsc::{TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use hardware_scanner::{
    HardwareInfo, Hardwarescanner,
    model::{ScanError, StaticInfo},
};

use crate::error::{ScanThreadError, ScanThreadResult, ThreadJoinError};

type Sender = sync::mpsc::SyncSender<Message>;
type Receiver = sync::mpsc::Receiver<Message>;

/// What the scan thread sends to the UI. Over-the-wire order is: an optional
/// [`GpuUnavailble`](Message::GpuUnavailble), then [`First`](Message::First), then an endless
/// stream of [`Info`](Message::Info).
#[derive(Debug)]
pub enum Message {
    /// A regular scan. If the UI has not read the previous one yet this message is dropped, since
    /// the next scan replaces it anyway.
    Info(HardwareInfo),
    /// Sent once: the first scan plus the hardware facts that never change.
    First {
        info: HardwareInfo,
        static_info: StaticInfo,
    },
    /// The scanner could not start at all; nothing else follows.
    StartupError(ScanError),
    /// No usable GPU, with the reason. Scanning goes on without GPU data.
    GpuUnavailble(ScanError),
}

/// Handle to the scan thread. Dropping it closes the channel, which makes the thread stop.
pub struct ScannerThread {
    receiver: Receiver,
    handle: JoinHandle<()>,
}
impl ScannerThread {
    /// Starts scanning every `scan_interval`. `redraw_callback` runs after each message so the
    /// UI wakes up and reads it.
    pub fn spawn<F: Fn() + Send + 'static>(scan_interval: Duration, redraw_callback: F) -> Self {
        let (sender, receiver) = sync::mpsc::sync_channel(1);
        let handle = thread::spawn(move || run(sender, scan_interval, redraw_callback));
        Self { handle, receiver }
    }

    /// The next message if one is waiting, `Ok(None)` if not, and an error once the thread is
    /// gone. Never blocks.
    pub fn recieve(&self) -> ScanThreadResult<Option<Message>> {
        match self.receiver.try_recv() {
            Ok(message) => Ok(Some(message)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(err @ TryRecvError::Disconnected) => Err(ScanThreadError::ThreadRecieve(err)),
        }
    }

    /// Waits for the thread to end. Only returns when the thread has stopped or panicked.
    pub fn join(self) -> ScanThreadResult {
        self.handle
            .join()
            .map_err(|e| ThreadJoinError::new(e).into())
    }
}

/// The thread body: start the scanner, send the one-shot messages, then scan forever.
fn run<F: Fn() + Send + 'static>(sender: Sender, scan_interval: Duration, redraw_callback: F) {
    use std::mem::take;
    let mut scanner = match Hardwarescanner::new() {
        Ok(val) => val,
        Err(err) => {
            send_blocking(&sender, Message::StartupError(err));
            redraw_callback();
            return;
        }
    };

    let mut gpu_error = None;
    let mut info = scanner.initinalize(&mut gpu_error);
    if let Some(err) = gpu_error {
        if send_blocking(&sender, Message::GpuUnavailble(err)) {
            return;
        }
        // The UI only drains the channel when it repaints.
        redraw_callback();
    }

    let static_info = scanner.static_info();
    let first = Message::First {
        info: take(&mut info),
        static_info,
    };
    if send_blocking(&sender, first) {
        return;
    }
    redraw_callback();

    loop {
        scanner.scan(&mut info);
        if send(&sender, Message::Info(take(&mut info))) {
            return;
        }
        redraw_callback();
        thread::sleep(scan_interval);
    }
}

/// For one-shot messages that must arrive: waits for room in the channel.
/// Returns true when the receiver is gone.
fn send_blocking(sender: &Sender, message: Message) -> bool {
    sender.send(message).is_err()
}

/// For per-tick updates: if the UI hasn't caught up the tick is dropped, the next one replaces it.
/// Returns true when the receiver is gone.
fn send(sender: &Sender, message: Message) -> bool {
    match sender.try_send(message) {
        Ok(_) | Err(TrySendError::Full(_)) => false,
        Err(TrySendError::Disconnected(_)) => true,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn info() -> Message {
        Message::Info(HardwareInfo::default())
    }

    #[test]
    fn send_delivers_when_there_is_room() {
        let (sender, receiver) = sync::mpsc::sync_channel(1);

        assert!(!send(&sender, info()));
        assert!(matches!(receiver.try_recv(), Ok(Message::Info(_))));
    }

    #[test]
    fn send_drops_the_tick_when_the_channel_is_full() {
        let (sender, receiver) = sync::mpsc::sync_channel(1);

        assert!(!send(&sender, info()));
        // full now: this one is dropped, and the call neither blocks nor reports a closed channel
        assert!(!send(&sender, info()));

        assert!(matches!(receiver.try_recv(), Ok(Message::Info(_))));
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn send_reports_a_closed_receiver() {
        let (sender, receiver) = sync::mpsc::sync_channel(1);
        drop(receiver);

        assert!(send(&sender, info()));
    }

    #[test]
    fn send_blocking_reports_a_closed_receiver() {
        let (sender, receiver) = sync::mpsc::sync_channel(1);
        drop(receiver);

        assert!(send_blocking(&sender, info()));
    }

    #[test]
    fn send_blocking_waits_for_room_instead_of_dropping() {
        let (sender, receiver) = sync::mpsc::sync_channel(1);
        sender.try_send(info()).unwrap();

        let reader = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            receiver.recv().unwrap();
            receiver
        });

        let start = Instant::now();
        assert!(!send_blocking(&sender, info()));
        assert!(start.elapsed() >= Duration::from_millis(50), "returned without waiting");

        // the second message was kept, not dropped
        let receiver = reader.join().unwrap();
        assert!(matches!(receiver.try_recv(), Ok(Message::Info(_))));
    }

    /// Regression test: `First` used to be sent with `try_send`, so right after a GPU note it
    /// hit the one-slot channel while it was full and was silently dropped.
    #[test]
    fn the_first_message_arrives_and_carries_static_info() {
        let scanner_thread = ScannerThread::spawn(Duration::from_millis(50), || {});
        let deadline = Instant::now() + Duration::from_secs(30);

        while Instant::now() < deadline {
            match scanner_thread.recieve().expect("the scan thread should be running") {
                Some(Message::First { static_info, .. }) => {
                    assert!(static_info.cpu.logical_cores > 0);
                    return;
                }
                Some(Message::StartupError(error)) => panic!("scanner failed to start: {error}"),
                // a GPU note or an early scan: keep reading
                Some(_) => {}
                None => thread::sleep(Duration::from_millis(20)),
            }
        }
        panic!("no First message within 30 seconds");
    }
}
