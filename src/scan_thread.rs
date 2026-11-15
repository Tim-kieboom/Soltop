use std::{
    sync::{
        self,
        mpsc::{TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use hardware_scanner::{HardwareInfo, Hardwarescanner, model::ScanError};

use crate::error::{ScanThreadError, ScanThreadResult, ThreadJoinError};

type Sender = sync::mpsc::SyncSender<Message>;
type Receiver = sync::mpsc::Receiver<Message>;

#[derive(Debug)]
pub enum Message {
    Info(HardwareInfo),
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
            send(&sender, Message::StartupError(err));
            return;
        }
    };

    let mut gpu_error = None;
    let mut info = scanner.initinalize(&mut gpu_error);
    if let Some(err) = gpu_error {
        if send(&sender, Message::GpuUnavailble(err)) {
            return;
        }
    }

    loop {
        scanner.scan(&mut info);
        if send(&sender, Message::Info(take(&mut info))) {
            return;
        }
        redraw_callback();
        thread::sleep(scan_interval);
    }
}

fn send(sender: &Sender, message: Message) -> bool {
    match sender.try_send(message) {
        Ok(_) | Err(TrySendError::Full(_)) => false,
        Err(TrySendError::Disconnected(_)) => true,
    }
}
