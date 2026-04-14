# Business Gaps & Questions

## Question: How to handle expired matches?
- **Hypothesis:** Auto-cancel after 3 mins per prompt.
- **Solution:** Implemented background cleanup logic.

---

## Additional Questions

1. **How to ensure offline reliability for all workflows?**
	- Hypothesis: All data and state transitions are persisted locally with checkpoint-based recovery.
	- Solution: Use SQLite for all business data, implement crash recovery and state checkpointing.

2. **How to enforce role-based access and data scope?**
	- Hypothesis: Role and tenant-based access control with local enforcement.
	- Solution: Implement local RBAC, scope queries and UI by role/tenant, enforce at API and UI layers.

3. **How to manage large document/video uploads offline?**
	- Hypothesis: Chunked/resumable uploads with local file system storage.
	- Solution: Store files in 25MB chunks, support resume after restart, use local file system APIs.

4. **How to handle claim timeouts and re-open logic?**
	- Hypothesis: Claims auto-cancel after 72 hours, can be reopened once with manager approval.
	- Solution: Implement claim state machine with timeout logic and manager override for reopen.

5. **How to ensure auditability and compliance?**
	- Hypothesis: All actions are logged with user, timestamp, and context.
	- Solution: Local audit log with export, visible watermarking on downloads, versioned document actions.

6. **How to support offline updates and rollback?**
	- Hypothesis: Updates are imported from signed USB packages, rollback to previous version is supported.
	- Solution: Implement offline update/rollback logic, verify signatures, maintain previous version for rollback.
