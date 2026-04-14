//! Enumerated capabilities. Add new variants here and wire them into
//! `roles::Role::permissions()` — never grant via ad-hoc string checks.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    // Administration
    ConfigureRules,
    ConfigureTemplates,
    ConfigurePermissions,
    ManageUsers,

    // Claims & settlements
    ApproveSettlement,
    ReopenClaim,
    ViewClaim,

    // Operations
    ParcelOperate,
    AcceptResidentSubmission,

    // Resident data
    InputResidentData,
    ViewResidentData,

    // Generic read / export
    ReadAny,
    ExportReport,

    // Audit
    AuditLogRead,
}

impl Permission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::ConfigureRules => "configure_rules",
            Permission::ConfigureTemplates => "configure_templates",
            Permission::ConfigurePermissions => "configure_permissions",
            Permission::ManageUsers => "manage_users",
            Permission::ApproveSettlement => "approve_settlement",
            Permission::ReopenClaim => "reopen_claim",
            Permission::ViewClaim => "view_claim",
            Permission::ParcelOperate => "parcel_operate",
            Permission::AcceptResidentSubmission => "accept_resident_submission",
            Permission::InputResidentData => "input_resident_data",
            Permission::ViewResidentData => "view_resident_data",
            Permission::ReadAny => "read_any",
            Permission::ExportReport => "export_report",
            Permission::AuditLogRead => "audit_log_read",
        }
    }
}
