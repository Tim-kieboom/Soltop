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

#[derive(Debug)]
pub enum Message {
    Info(HardwareInfo),
    /// Sent once: the first scan plus the hardware facts that never change.
    First {
        info: HardwareInfo,
        static_info: StaticInfo,
    },
    StartupError(ScanError),
    GpuUnavailble(ScanError),
}

pub struct ScannerThread {
    receiver: Receiver,
    handle: JoinHandle<()>,
}
impl ScannerThread {
    pub fn spawn<F: Fn() + Send + 'static>(scan_interval: Duration, redraw_callback: F) -> Self {
        let (sender, receiver) = sync::mpsc::sync_channel(1);
        let handle = thread::spawn(move || run(sender, scan_interval, redraw_callback));
        Self { handle, receiver }
    }

    pub fn recieve(&self) -> ScanThreadResult<Option<Message>> {
        match self.receiver.try_recv() {
            Ok(message) => Ok(Some(message)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(err @ TryRecvError::Disconnected) => Err(ScanThreadError::ThreadRecieve(err)),
        }
    }

    pub fn join(self) -> ScanThreadResult {
        self.handle
            .join()
            .map_err(|e| ThreadJoinError::new(e).into())
    }
}

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
