use crossbeam::channel::{Receiver, Sender, TryRecvError};

use crate::{Pusher, Queue, Record};

/// Struct containing a record being processed from the terminal
///
/// Note: Technically, this packet is Clone, meaning multiple processors
///       could process and attempt and complete this packet.
pub struct Packet {
    /// Record needing processing
    pub record: Record,
    /// Sender to complete the processing
    pub send: Sender<Record>,
}

/// Terminal manages a single record and an input/output queue
#[derive(Debug)]
pub struct Terminal {
    /// Inner state
    state: State,
    /// Input queue to schedule records for processing
    input: Queue<Record>,
    /// Pusher to output records after processing
    output: Pusher<Record>,
    /// Retries
    output_retries: usize,
    /// Packet drops
    packet_drops: usize,
}

impl Terminal {
    /// Creates a new terminal
    ///
    /// Returns the terminal and the output queue the terminal is connected to
    #[inline]
    pub fn new() -> (Self, Queue<Record>) {
        Self::create(Queue::new())
    }

    /// Creates a connected terminal
    ///
    /// Returns a terminal w/ specified i/o configuration
    #[inline]
    pub fn connect(input: Queue<Record>, output: Pusher<Record>) -> Self {
        Self {
            state: State::Empty,
            input,
            output,
            output_retries: 0,
            packet_drops: 0,
        }
    }

    /// Creates a terminal from components
    #[inline]
    pub fn from_parts(current: State, input: Queue<Record>, output: Pusher<Record>) -> Self {
        Self {
            state: current,
            input,
            output,
            output_retries: 0,
            packet_drops: 0,
        }
    }

    /// Splits the terminal into it's components
    #[inline]
    pub fn to_parts(self) -> (State, Queue<Record>, Pusher<Record>) {
        (self.state, self.input, self.output)
    }

    /// Advances the terminal's internal state,
    ///
    /// Returns true if a record is currently available inside of the terminal for processing
    #[inline]
    pub fn advance(&mut self) {
        match self.state.step(&self.input, &self.output) {
            Step::Retry => {
                self.output_retries += 1;
            }
            Step::PacketDrop => {
                self.packet_drops += 1;
            }
            Step::Reset => {
                self.output_retries = 0;
                self.packet_drops = 0;
            }
            Step::Continue => {}
        }
    }

    /// Returns the mext packet, or None if no packet is available
    #[inline]
    pub fn packet(&mut self) -> Option<Packet> {
        self.state
            .process()
            .map(|(record, send)| Packet { record, send })
    }

    /// Returns a input Pusher,
    ///
    /// Returns None if the input queue has been closed
    #[inline]
    pub fn input(&self) -> Option<Pusher<Record>> {
        self.input.pusher()
    }

    /// Returns the output push retry count
    #[inline]
    pub fn output_retries(&self) -> usize {
        self.output_retries
    }

    /// Returns the packet process drop count
    #[inline]
    pub fn packet_drops(&self) -> usize {
        self.packet_drops
    }

    #[inline]
    fn create(input: Queue<Record>) -> (Self, Queue<Record>) {
        let output = Queue::new();
        (
            Self::connect(input, output.pusher().expect("should exist just created")),
            output,
        )
    }
}

/// Variants of Terminal State
#[derive(Debug, Default)]
pub enum State {
    /// State is empty
    #[default]
    Empty,
    /// State contains a new record
    New(Record),
    /// State contains parts to receive a record that has finished evaluation
    Pending {
        ready: Receiver<Record>,
        backup: Record,
    },
    /// State contains a record that is ready for output
    Ready(Record),
}

#[derive(Debug)]
enum Step {
    /// Record could not be sent to output and requires a retry
    Retry,
    /// Packet processing was dropped, backup will be restored
    PacketDrop,
    /// State should reset
    Reset,
    /// No-op
    Continue,
}

impl State {
    /// Updates state and returns the next step
    fn step(&mut self, input: &Queue<Record>, output: &Pusher<Record>) -> Step {
        loop {
            match self.take() {
                State::Ready(ready) => match output.push(ready) {
                    Some(retry) => {
                        *self = State::Ready(retry);
                        return Step::Retry;
                    }
                    None => {
                        *self = input.pop().map(State::New).unwrap_or_default();
                        return Step::Reset;
                    }
                },
                State::Pending { ready, backup } => match ready.try_recv() {
                    Ok(ready) => {
                        *self = State::Ready(ready);
                        continue; // Loop once transition the ready record and attempt to output
                    }
                    Err(TryRecvError::Empty) => {
                        *self = State::Pending { ready, backup };
                        return Step::Continue;
                    }
                    Err(TryRecvError::Disconnected) => {
                        *self = State::New(backup);
                        return Step::PacketDrop;
                    }
                },
                State::Empty => {
                    *self = input.pop().map(State::New).unwrap_or_default();
                    return Step::Continue;
                }
                State::New(record) => {
                    *self = State::New(record);
                    return Step::Continue;
                }
            }
        }
    }

    /// Takes the inner state
    #[inline]
    fn take(&mut self) -> State {
        std::mem::take(self)
    }

    /// Returns the current record and a sender to complete processing
    ///
    /// Once the operator has completed its processing, and sends the record
    /// back to the terminal, the terminal transitions the entry to a ready
    /// state
    ///
    /// Otherwise, returns None the current state cannot be processed
    #[inline]
    fn process(&mut self) -> Option<(Record, Sender<Record>)> {
        let entry = self.take();
        match entry {
            State::Empty => None,
            State::New(record) => {
                let (tx, rx) = crossbeam::channel::bounded(1);
                let _ = std::mem::replace(
                    self,
                    Self::Pending {
                        ready: rx,
                        backup: record.clone(),
                    },
                );
                Some((record, tx))
            }
            noop => {
                let _ = std::mem::replace(self, noop);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Namespace, Queue, Record,
        wire::{Terminal, terminal::Packet},
    };

    #[test]
    fn test_connect_state_machine() {
        let input = Queue::<Record>::new();
        let output = Queue::<Record>::new();
        let mut terminal = Terminal::connect(input, output.pusher().unwrap());
        
        // Test pushing a record to the terminal input queue
        terminal
            .input()
            .unwrap()
            .push(Namespace::ephemeral().commit("test", b"hello world"));
        terminal.advance();

        // Test getting a packet from the terminal
        let Packet { record, send } = terminal.packet().unwrap();
        send.send(record).unwrap();

        // Test advancing the terminal state
        terminal.advance();
        output.pop().unwrap();
    }
}
