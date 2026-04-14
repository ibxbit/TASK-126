//! Parcel lifecycle — configurable state machine, transition
//! validator, and immutable history.

pub mod machine;
pub mod state;
pub mod transition;

pub use machine::{guard_for, GuardCode, GuardFn, StateMachine, StateMachineError, TransitionRule};
pub use state::ParcelState;
pub use transition::{
    ParcelRepository, TransitionInput, TransitionRecord, TransitionRepository,
};
