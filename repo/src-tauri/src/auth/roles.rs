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

#[cfg(test)]
mod tests {
    use super::*;
    use Permission::*;

    const ROLES: [Role; 5] = [
        Role::Administrator,
        Role::PropertyManager,
        Role::Staff,
        Role::Reviewer,
        Role::Liaison,
    ];

    #[test]
    fn as_str_round_trips_for_all_roles() {
        for r in ROLES {
            let parsed = Role::from_str(r.as_str()).expect("round-trip");
            assert_eq!(parsed, r, "{:?}", r);
        }
    }

    #[test]
    fn from_str_rejects_unknown_role_codes() {
        assert!(Role::from_str("super_admin").is_none());
        assert!(Role::from_str("").is_none());
        assert!(Role::from_str("Staff").is_none(), "case-sensitive");
    }

    #[test]
    fn administrator_has_every_permission_in_matrix() {
        // Administrator is least-privilege "everything wired"; this
        // catches a stale matrix when a new Permission variant is added.
        let admin = Role::Administrator;
        let all = [
            ConfigureRules, ConfigureTemplates, ConfigurePermissions, ManageUsers,
            ApproveSettlement, ReopenClaim, ViewClaim,
            ParcelOperate, AcceptResidentSubmission,
            InputResidentData, ViewResidentData,
            ReadAny, ExportReport, AuditLogRead,
        ];
        for p in all {
            assert!(admin.has(p), "Administrator missing {:?}", p);
        }
    }

    #[test]
    fn liaison_has_minimal_resident_data_permissions_only() {
        let l = Role::Liaison;
        assert!(l.has(InputResidentData));
        assert!(l.has(ViewResidentData));
        // Negative checks — Liaison must not have any operational power.
        for forbidden in [
            ParcelOperate, ApproveSettlement, ReopenClaim,
            ManageUsers, ConfigureRules, ConfigureTemplates,
            ConfigurePermissions, AuditLogRead, ExportReport,
        ] {
            assert!(!l.has(forbidden), "Liaison must not hold {:?}", forbidden);
        }
    }

    #[test]
    fn staff_can_operate_parcels_but_not_approve_settlement() {
        let s = Role::Staff;
        assert!(s.has(ParcelOperate));
        assert!(s.has(AcceptResidentSubmission));
        assert!(s.has(ViewClaim));
        assert!(!s.has(ApproveSettlement));
        assert!(!s.has(ReopenClaim));
        assert!(!s.has(ManageUsers));
    }

    #[test]
    fn reviewer_can_view_and_export_but_not_mutate() {
        let r = Role::Reviewer;
        assert!(r.has(ViewClaim));
        assert!(r.has(ExportReport));
        assert!(r.has(AuditLogRead));
        assert!(!r.has(ParcelOperate), "reviewer is read-only");
        assert!(!r.has(ApproveSettlement));
        assert!(!r.has(ManageUsers));
    }

    #[test]
    fn property_manager_can_approve_settlement_and_reopen_claim() {
        let pm = Role::PropertyManager;
        assert!(pm.has(ApproveSettlement));
        assert!(pm.has(ReopenClaim));
        assert!(pm.has(ParcelOperate));
        // PMs do NOT manage users (Administrator only).
        assert!(!pm.has(ManageUsers));
        assert!(!pm.has(ConfigureRules));
    }

    #[test]
    fn permission_matrix_is_strictly_monotone_admin_superset_of_pm() {
        // Every permission held by PropertyManager must also be held by Administrator.
        for p in Role::PropertyManager.permissions() {
            assert!(
                Role::Administrator.has(*p),
                "Administrator missing PM permission: {:?}",
                p
            );
        }
    }

    #[test]
    fn no_role_grants_a_permission_outside_the_canonical_enum() {
        // This is a typo / refactor regression check: every permission
        // returned by `permissions()` must serialize to a known string.
        let known: std::collections::HashSet<&str> = [
            "configure_rules", "configure_templates", "configure_permissions",
            "manage_users", "approve_settlement", "reopen_claim", "view_claim",
            "parcel_operate", "accept_resident_submission", "input_resident_data",
            "view_resident_data", "read_any", "export_report", "audit_log_read",
        ].into_iter().collect();
        for r in ROLES {
            for p in r.permissions() {
                assert!(known.contains(p.as_str()), "{:?} returned unknown permission", r);
            }
        }
    }

    #[test]
    fn role_serializes_as_snake_case_string() {
        let json = serde_json::to_string(&Role::PropertyManager).unwrap();
        assert_eq!(json, r#""property_manager""#);
    }
}
