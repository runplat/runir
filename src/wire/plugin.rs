// impl<R: IRecord + IPlugin + Sized> Wire<R> {
//     /// Returns the plugin driver if supported by the wire record
//     #[inline]
//     pub fn plugin(&self) -> Plugin {
//         let record = self.record.to_record();
//     }
// }
use crate::{IRecord, Opts, Record, Worker, wire::Wire};

/// Plugin system
///
/// To drive the wire runtime, the runtime must resolve the transport flag set on each record entering the system.
///
/// The runtime is composed of a set of plugins. Each plugin is responsible for reconciling records assigned to them.
///
/// At a minimum, to remove the transport branch flag, the record must be in a non-transport state.
///
/// What is a non-transport state? At a minimum, it is the availability of the bytes described by the wire unit.
///
/// However, a non-transport state can be defined by a plugin, which means the definition is not limited to the minimum state.
///
/// It could also be something like the availability of additional state associated to the record. ie. lfs frontend
///
/// ---
pub trait IPlugin {
    /// Record mutation plugin function
    #[inline]
    fn record_mut(record: Record) -> Record {
        record
    }

    /// Opts mutation plugin function
    #[inline]
    fn opts_mut(opts: Opts) -> Opts {
        opts
    }
}

fn done(mut record: Record) -> Record {
    if record.is_valid() && record.opts().is_transport() {
        record.opts.done();
    }
    record
}

/// Type-aluas for a wire "bus"
pub type Bus = Option<Record>;

type _State<T> = statig::Outcome<T>;

const READY_STATE: _State<State> = statig::Outcome::Transition(State::Ready {});
const PENDING_STATE: _State<State> = statig::Outcome::Transition(State::Pending {});

#[statig::state_machine(initial = "State::init()")]
impl Wire<Bus> {
    #[superstate]
    fn filtering(&mut self, context: &mut Worker) -> statig::Outcome<State> {
        match &mut self.record {
            Bus::None => READY_STATE,
            Bus::Some(record) => {
                match self.runtime.filters.get(record.opts()) {
                    Some(filters) => {
                        let next = filters.iter().fold(record.to_record(), |r, f| (*f)(r));
                        let _prev = std::mem::replace(record, next);
                    }
                    None => {
                        let next = done(record.to_record());
                        let _prev = std::mem::replace(record, next);
                    }
                }

                let wire = Wire::from(record.to_record());
                if wire.is_transport_complete() {
                    if context.push(wire.record.clone()) {
                        READY_STATE
                    } else {
                        PENDING_STATE
                    }
                } else {
                    PENDING_STATE
                }
            }
        }
    }

    #[state(super = filter)]
    fn init(&self) -> statig::Outcome<State> {
        statig::Outcome::Super
    }

    #[state(super = filter)]
    fn pending(&self) -> statig::Outcome<State> {
        statig::Outcome::Super
    }

    #[state(super = filter)]
    fn ready(&mut self, context: &mut Worker) -> statig::Outcome<State> {
        if self.settings.fsm_ready_sync_enable
            && context.count() >= self.settings.fsm_ready_sync_threshold
        {
            match context.sync() {
                Ok(_fut) => {
                    // self.fsm_ready_sync_fut_list.push(profile!(_fut));
                }
                Err(_) => {
                    // This means that the worker was not configured
                    /*
                        Convert to an index?
                    */
                }
            }
        }
        statig::Outcome::Super
    }
}

// impl Wire<Bus> {
//     /// Swaps the bus and returns the previous bus
//     #[inline]
//     fn swap_bus(&mut self) -> Bus {
//         let next: Bus = self.bus_in.pop().map(|v| Bus::Some(v)).unwrap_or_default();
//         let record: &mut Bus = &mut self.record;
//         std::mem::replace::<Bus>(record, next)
//     }
// }

#[cfg(test)]
mod tests {
    use crate::{
        Namespace, Opts,
        frontend::state::SharedState,
        wire::{
            Wire,
            plugin::{Bus, IPlugin},
        },
    };
    use statig::prelude::*;

    struct TestFilter;

    impl IPlugin for TestFilter {
        fn record_mut(record: crate::Record) -> crate::Record {
            record
        }
    }

    #[test]
    fn test_fsm() {
        let state = SharedState::default();
        let mut worker = state.store().worker();
        let worker = &mut worker;

        let record = Namespace::ephemeral().commit("test", b"test").transport();
        let fsm = Wire::from(Bus::Some(record));

        fsm.filter::<TestFilter>(Opts::ephemeral());

        let mut fsm = fsm.state_machine();
        fsm.step_with_context(worker);
    }
}
