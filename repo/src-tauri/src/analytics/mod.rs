//! Local analytics framework: typed events, dashboards, exports,
//! A/B testing.

pub mod dashboard;
pub mod events;
pub mod experiments;
pub mod exports;

pub use dashboard::{
    compute_funnel, compute_quality, compute_retention, DashboardError, FunnelDefinition,
    FunnelResult, FunnelStepResult, QualityMetrics, RetentionCohort, RetentionResult,
};
pub use events::{
    track_event, EventCategory, EventInput, EventRepository, TrackError, TrackedEvent,
};
pub use experiments::{
    assign_variant, decide_variant, AssignmentError, Experiment, ExperimentRepository, Variant,
    VariantAssignment,
};
pub use exports::{to_csv, to_json_lines, ExportError};
