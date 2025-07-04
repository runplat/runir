use crate::Record;

pub struct SyncThread {
    to_pusher: (
        crossbeam::channel::Sender<Vec<Record>>,
        crossbeam::channel::Receiver<Vec<Record>>,
    ),
}

impl SyncThread {
    /// Starts the sync thread
    ///
    /// The sync thread watches for Records to process
    #[inline]
    pub fn start(self) -> std::io::Result<std::thread::JoinHandle<()>> {
        // Dedicated thread for the sync thread to operate on
        std::thread::Builder::new()
            .name("bg-sync".to_string())
            .spawn(|| {
                let thread = self;
                let recv_work = thread.to_pusher.1;
                loop {
                    crossbeam::select! {
                        recv(recv_work) -> work => {
                            match work {
                                Ok(records) => {
                                    
                                },
                                Err(_) => break,
                            }
                        }
                    }
                }
            })
    }
}
