# API Specification: Shoreline Property Operations Console

## Overview
All APIs are local (desktop app) and interact with a local SQLite database and file system. APIs are exposed via IPC (Tauri commands) and are not network-accessible.

---

## Authentication & Access Control
- **Login**: Authenticate user, return session token, enforce role/tenant scope.
- **GetCurrentUser**: Returns user profile, roles, permissions.
- **Logout**: Ends session, clears sensitive state.

---

## Resident & Profile Management
- **ListResidents**: Query residents by name, unit, status.
- **GetResidentProfile**: Fetch full profile, preferences, documents.
- **UpdateResidentProfile**: Edit resident info, preferences.

---

## Parcel Lifecycle Management
- **ListParcels**: Filter by state, location, resident, date.
- **CheckInParcel**: Register new parcel, capture operator, timestamp, location, notes.
- **CheckOutParcel**: Mark as checked out, require prior check-in.
- **DeliverParcel**: Mark as delivered, validate state.
- **ReturnParcel**: Mark as returned/exception, add notes.
- **ConfirmReceipt**: Resident confirms receipt, logs timestamp.

---

## Claims & Matching Workflow
- **ListClaims**: Filter by status, resident, category.
- **SubmitClaim**: Create new claim, attach proofs.
- **WithdrawClaim**: Withdraw open claim.
- **RespondToClaim**: Counterparty response, attach proofs.
- **ReopenClaim**: Manager approval required.
- **AutoCancelClaim**: Triggered after 72h timeout.

---

## Document & Attachment Management
- **ListDocuments**: Query by resident, parcel, claim, tag.
- **UploadDocument**: Chunked upload, resumable, versioned.
- **DownloadDocument**: Watermarked, password-protected if external share.
- **PreviewDocument**: Offline preview for supported formats.
- **TagDocument**: Add/remove tags.

---

## Settlement & Deposit Management
- **ListSettlements**: Filter by resident, status, date.
- **CreateSettlement**: Start new move-out/deposit settlement.
- **AddDeduction**: Add deduction line-item (amount, category, evidence).
- **ApproveSettlement**: Two-step approval, triggers payout.
- **GenerateStatement**: Produce printable statement.

---

## Scheduling & Staffing
- **ListSchedules**: Query by date, staff, type.
- **CreateSchedule**: Add inspection/staffing slot, apply rules.
- **UpdateSchedule**: Edit slot, enforce constraints.
- **VersionSchedules**: View/restore previous rule versions.

---

## Analytics & Events
- **RecordEvent**: Log impression/click/completion/conversion.
- **GetDashboardData**: Aggregate funnel, retention, quality metrics.
- **ExportAnalytics**: Export CSV/JSON.
- **ConfigureExperiment**: Set up A/B test, split, dates.

---

## System & Updates
- **GetSystemStatus**: App health, file handles, update status.
- **ImportUpdate**: Import signed update package, support rollback.
- **GetAuditLog**: Export audit actions, filter by user/date/action.
