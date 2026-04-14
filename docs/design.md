# Design Document: Shoreline Property Operations Console

## 1. Architecture Overview
- **Frontend:** React (Vite) desktop UI, multi-window, context menus, keyboard shortcuts, system tray integration.
- **Backend:** Rust (Tauri) for business logic, state machines, workflow engines, and IPC APIs.
- **Persistence:** Local SQLite database (encrypted fields), local file system for documents/attachments.

---

## 2. Key Modules
### a. Authentication & Access Control
	- Local login, role/tenant-based RBAC, session management.

### b. Resident & Profile Management
	- CRUD for residents, profile/preferences, document linking.

### c. Parcel Lifecycle Engine
	- Configurable state machine: check-in, check-out, delivery, receipt, return/exception.
	- Transition validation, operator/timestamp/location capture, audit logging.

### d. Claims & Matching Workflow
	- Rule-based matching (category, address, time, keywords).
	- Claim submission, withdrawal, two-party confirmation, auto-cancel/reopen logic.

### e. Document & Attachment Management
	- Chunked/resumable uploads, versioning, tagging, offline preview, watermarking, password-protected shares.

### f. Settlement & Deposit Management
	- Move-out/deposit tracking, inspections, deductions, approvals, statement generation, payout workflow.

### g. Scheduling Engine
	- Rule-based slot allocation, hard/soft constraints, versioned rules, enable/disable controls.

### h. Analytics & Event Tracking
	- Local event log, funnel/retention/quality metrics, dashboard, A/B experiment config, export.

### i. System & Updates
	- Crash recovery, file handle management, offline update/rollback, audit log export.

---

## 3. Security & Compliance
- AES encryption for sensitive fields, partial masking for identifiers.
- All downloads watermarked (username + timestamp).
- Audit log for all actions, exportable by admin.

---

## 4. Offline & Reliability Features
- All data and workflows persist locally (no network dependency).
- Checkpoint-based state recovery after crash.
- Offline update/rollback via signed USB package.

---

## 5. UI/UX Principles
- Desktop-first, multi-window, high-DPI support.
- Context menus for actions, global keyboard shortcuts.
- System tray for background timers/reminders.
- Responsive, accessible, English-only UI.
