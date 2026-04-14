//! Dispute resolution: claim lifecycle, two-party confirmation,
//! similarity matching, and timeout handling.

pub mod machine;
pub mod matching;
pub mod state;
pub mod timeout;

pub use machine::{
    apply_transition, ClaimEvent, ClaimLifecycleError, ClaimLifecycleMachine, ClaimRepository,
    TransitionOutcome,
};
pub use matching::{
    find_matches, normalize_address, tokenize_keywords, ClaimFeatures, MatchCandidate,
    MatchWeights,
};
pub use state::{ClaimKind, ClaimStatus, PartyRole, PartyResponse};
pub use timeout::{enforce_timeout_lazy, ClaimTimeoutScheduler, TimeoutError};
