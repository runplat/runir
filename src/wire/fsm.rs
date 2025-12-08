use std::path::PathBuf;

use crate::{
    Index, Queue, Record, store::StoreArchive, vol::Volume, wire::{Terminal, Wire, plugin::Bus}
};

/// Type-alias for a wire protocol finite state machine backend
///
/// Uses the "volume" abstraction where the target is defined as the Wire<Bus>,
/// and encoder as the Terminal
///
/// Terminal provides the input processing and output channel coordination and state management,
/// while the Wire<Bus> provides wire-record aware functions
type Backend = Volume<Wire<Bus>, Terminal>;

impl Backend {
    /// Receive a msg_tar from this backend
    #[inline]
    pub async fn receive(self, msg_tar: impl Into<PathBuf>) -> std::io::Result<()> {
        let store = StoreArchive::unpack(msg_tar.into()).await?;
        let members = store.members().filter_map(|m| m.get_records().ok());
        for mut input in members {
            match self.encoder().input() {
                Some(pusher) => {
                    for r in input.drain(..) {
                        pusher.ensure_push(r);
                    }
                }
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NetworkDown,
                        "Encoder input queue was not enabled",
                    ));
                }
            }
        }
        Ok(())
    }
}

type Frontend<R, S> = Volume<Queue<Record>, Index<R, S>>;

fn test() {
    let (terminal, output) = Terminal::new();
    let backend = Backend::from_parts((Wire::from(Bus::None), terminal));

    // TODO:
    let frontend = Frontend::<Record, Vec<Record>>::from_parts((output, Index::default()));

    let svc = (frontend, backend);

    // 1) Record is created
    // 2) Record is sent to backend terminal
    // 3) Record is proccessed and put on the wire bus
    // 4) Record is filtered on the bus and returned back to the terminal
    // 5) Record is sent to frontend index from terminal
    // 6) Record is indexed

    // 1) <- Plugin(Send)
    // 2) <- Plugin(Accept/Defer/Reject)
    // 3) <- Plugin(Process)
    // 4) <- Plugin(Filter)
    // 5) <- Plugin(Map)
    // 6) <- Plugin(Reduce)
}
