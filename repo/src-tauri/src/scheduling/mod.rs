//! Configurable scheduling engine: rule sets, constraint validation,
//! and slot allocation.

pub mod algorithm;
pub mod constraints;
pub mod rules;

pub use algorithm::{
    propose_schedule, Demand, Proposal, ProposedAssignment, ScheduleError, UnfulfilledDemand,
};
pub use constraints::{
    validate, Assignment, ConstraintReport, Severity, ViolationDetail,
};
pub use rules::{
    activate_version, DistributionMode, Rule, RuleKind, RuleRepository, RuleSet, RuleSetError,
    RuleSpec, TimeWindow,
};
