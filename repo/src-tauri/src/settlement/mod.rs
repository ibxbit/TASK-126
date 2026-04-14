//! Move-out and deposit settlement workflow.

pub mod approval;
pub mod payout;
pub mod statement;
pub mod workflow;

pub use approval::{
    approve_settlement, prepare_settlement, ApprovalError, ApprovalRecord, ApprovalStep,
};
pub use payout::{
    generate_check_request, post_ledger_for_settlement, CheckRequest, LedgerEntry, LedgerError,
    PayoutOutcome,
};
pub use statement::{
    generate_statement, render_statement_html, DeductionLine, SettlementStatement,
    StatementError,
};
pub use workflow::{
    apply_event, SettlementEvent, SettlementRepository, SettlementStatus, SettlementView,
    WorkflowError,
};
