import { describe, expect, it } from "vitest";
import type {
  RecoveryOutcome,
  HandleEntry,
  VerifiedPackageInfo,
  InstallOutcome,
  RollbackOutcome,
  InstalledVersion,
} from "./system";

describe("System IPC type contracts", () => {
  it("RecoveryOutcome covers all three variants", () => {
    const outcomes: RecoveryOutcome[] = [
      "clean_start",
      "unclean_repaired",
      "integrity_failed",
    ];
    expect(outcomes).toHaveLength(3);
  });

  it("HandleEntry can be constructed with all kinds", () => {
    const kinds: HandleEntry["kind"][] = ["file", "db_connection", "upload_chunk", "other"];
    const entries: HandleEntry[] = kinds.map((kind, i) => ({
      id: `h${i}`,
      kind,
      label: `handle-${kind}`,
      opened_at_unix: 1700000000 + i,
    }));
    expect(entries).toHaveLength(4);
    expect(entries[0].kind).toBe("file");
    expect(entries[3].kind).toBe("other");
  });

  it("VerifiedPackageInfo carries all expected fields", () => {
    const info: VerifiedPackageInfo = {
      package_id: "pkg-001",
      version: "1.2.0",
      created_at_unix: 1700000000,
      min_required_version: "1.0.0",
      notes: "Bug fixes and performance improvements",
    };
    expect(info.package_id).toBe("pkg-001");
    expect(info.version).toBe("1.2.0");
    expect(info.min_required_version).toBe("1.0.0");
  });

  it("VerifiedPackageInfo allows null optional fields", () => {
    const info: VerifiedPackageInfo = {
      package_id: "pkg-002",
      version: "2.0.0",
      created_at_unix: 1700000000,
      min_required_version: null,
      notes: null,
    };
    expect(info.min_required_version).toBeNull();
    expect(info.notes).toBeNull();
  });

  it("InstallOutcome contains version transition info", () => {
    const outcome: InstallOutcome = {
      previous_version: "1.0.0",
      new_version: "1.1.0",
      snapshot_path: "/backups/snap_v1",
      staging_path: "/staging/v1.1.0",
      restart_required: true,
    };
    expect(outcome.restart_required).toBe(true);
    expect(outcome.new_version).toBe("1.1.0");
  });

  it("RollbackOutcome contains version transition info", () => {
    const outcome: RollbackOutcome = {
      from_version: "1.1.0",
      to_version: "1.0.0",
      restart_required: true,
    };
    expect(outcome.from_version).toBe("1.1.0");
    expect(outcome.to_version).toBe("1.0.0");
  });

  it("InstalledVersion tracks active state", () => {
    const versions: InstalledVersion[] = [
      {
        id: "v1",
        version: "1.1.0",
        package_id: "pkg-001",
        installed_at_unix: 1700000001,
        is_active: true,
        snapshot_path: "/backups/snap_v1",
      },
      {
        id: "v0",
        version: "1.0.0",
        package_id: null,
        installed_at_unix: 1700000000,
        is_active: false,
        snapshot_path: null,
      },
    ];
    const active = versions.filter((v) => v.is_active);
    expect(active).toHaveLength(1);
    expect(active[0].version).toBe("1.1.0");
  });
});
