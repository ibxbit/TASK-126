//! Role definitions and the static permission matrix.

use serde::{Deserialize, Serialize};

use crate::auth::permissions::Permission;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Administrator,
    PropertyManager,
    Staff,
    Reviewer,
    Liaison,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Administrator => "administrator",
            Role::PropertyManager => "property_manager",
            Role::Staff => "staff",
            Role::Reviewer => "reviewer",
            Role::Liaison => "liaison",
        }
    }

    pub fn from_str(s: &str) -> Option<Role> {
        match s {
            "administrator" => Some(Role::Administrator),
            "property_manager" => Some(Role::PropertyManager),
            "staff" => Some(Role::Staff),
            "reviewer" => Some(Role::Reviewer),
            "liaison" => Some(Role::Liaison),
            _ => None,
        }
    }

    /// Static permission matrix. Least-privilege: a role holds ONLY the
    /// permissions explicitly listed here.
    pub fn permissions(&self) -> &'static [Permission] {
        use Permission::*;
        match self {
            Role::Administrator => &[
                ConfigureRules,
                ConfigureTemplates,
                ConfigurePermissions,
                ManageUsers,
                ApproveSettlement,
                ReopenClaim,
                ViewClaim,
                ParcelOperate,
                AcceptResidentSubmission,
                InputResidentData,
                ViewResidentData,
                ReadAny,
                ExportReport,
                AuditLogRead,
            ],
            Role::PropertyManager => &[
                ApproveSettlement,
                ReopenClaim,
                ViewClaim,
                ParcelOperate,
                AcceptResidentSubmission,
                InputResidentData,
                ViewResidentData,
                ReadAny,
                ExportReport,
                AuditLogRead,
            ],
            Role::Staff => &[
                ParcelOperate,
                AcceptResidentSubmission,
                ViewClaim,
                ViewResidentData,
                ReadAny,
            ],
            Role::Reviewer => &[
                ViewClaim,
                ViewResidentData,
                ReadAny,
                ExportReport,
                AuditLogRead,
            ],
            Role::Liaison => &[InputResidentData, ViewResidentData],
        }
    }

    pub fn has(&self, perm: Permission) -> bool {
        self.permissions().iter().any(|p| *p == perm)
    }
}
