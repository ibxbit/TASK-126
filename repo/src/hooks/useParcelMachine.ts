// React hook that surfaces the parcel lifecycle to a detail view:
// which transition buttons to show, how to apply one, and how to
// refresh the history log after a change.

import { useCallback, useEffect, useState } from "react";
import {
  availableTransitions,
  parcelHistory,
  ParcelState,
  transitionParcel,
  TransitionInput,
  TransitionRecord,
} from "../ipc/parcel";

export interface UseParcelMachine {
  current: ParcelState | null;
  available: ParcelState[];
  history: TransitionRecord[];
  loading: boolean;
  error: string | null;
  apply: (
    to: ParcelState,
    location: string,
    notes?: string,
  ) => Promise<TransitionRecord | null>;
  refresh: () => Promise<void>;
}

export function useParcelMachine(
  tenantId: string,
  parcelId: string,
  initialState: ParcelState | null,
): UseParcelMachine {
  const [current, setCurrent] = useState<ParcelState | null>(initialState);
  const [available, setAvailable] = useState<ParcelState[]>([]);
  const [history, setHistory] = useState<TransitionRecord[]>([]);
  const [loading, setLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [avail, hist] = await Promise.all([
        availableTransitions(tenantId, current),
        parcelHistory(parcelId),
      ]);
      setAvailable(avail);
      setHistory(hist);
    } catch (e) {
      setError(stringifyError(e));
    } finally {
      setLoading(false);
    }
  }, [tenantId, parcelId, current]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const apply = useCallback(
    async (to: ParcelState, location: string, notes?: string) => {
      setLoading(true);
      setError(null);
      try {
        const input: TransitionInput = {
          parcel_id: parcelId,
          tenant_id: tenantId,
          to_state: to,
          location,
          notes: notes ?? null,
        };
        const rec = await transitionParcel(input);
        setCurrent(rec.to_state);
        await refresh();
        return rec;
      } catch (e) {
        setError(stringifyError(e));
        return null;
      } finally {
        setLoading(false);
      }
    },
    [parcelId, tenantId, refresh],
  );

  return { current, available, history, loading, error, apply, refresh };
}

function stringifyError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object") {
    const obj = e as { message?: string; type?: string };
    if (obj.message) return obj.message;
    if (obj.type) return obj.type;
    try { return JSON.stringify(e); } catch { /* fall through */ }
  }
  return "Unknown error";
}
