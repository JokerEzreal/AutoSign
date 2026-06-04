import { createContext, useContext, useEffect, useState, ReactNode } from "react";
import { get } from "./api";

export interface AccountInfo {
  id: number;
  account_name: string;
  student_id_masked: string;
  balance_cents: number;
  auto_sign: boolean;
  status: string;
  modules: string[];
  last_synced_at: string | null;
}

export interface Me {
  role: string;
  account: AccountInfo | null;
}

interface MeCtx {
  me: Me | null;
  loading: boolean;
  reload: () => Promise<void>;
}

const Ctx = createContext<MeCtx>({ me: null, loading: true, reload: async () => {} });

export function MeProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null>(null);
  const [loading, setLoading] = useState(true);

  const reload = async () => {
    try {
      const data = await get<Me>("/api/me");
      setMe(data);
    } catch {
      setMe(null);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    reload();
  }, []);

  return <Ctx.Provider value={{ me, loading, reload }}>{children}</Ctx.Provider>;
}

export const useMe = () => useContext(Ctx);
