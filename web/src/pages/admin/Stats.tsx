import { useEffect, useState } from "react";
import { get, yuan } from "../../api";

interface S {
  total_accounts: number;
  active_accounts: number;
  today_signs: number;
  today_revenue_cents: number;
}

export default function Stats() {
  const [s, setS] = useState<S | null>(null);
  useEffect(() => {
    get<S>("/api/admin/stats").then(setS);
  }, []);

  return (
    <div>
      <h2 className="page-title">统计看板</h2>
      <div className="grid">
        <div className="stat">
          <div className="label">总账号</div>
          <div className="value">{s?.total_accounts ?? "—"}</div>
        </div>
        <div className="stat">
          <div className="label">活跃账号</div>
          <div className="value">{s?.active_accounts ?? "—"}</div>
        </div>
        <div className="stat">
          <div className="label">今日成功签到</div>
          <div className="value">{s?.today_signs ?? "—"}</div>
        </div>
        <div className="stat">
          <div className="label">今日营收</div>
          <div className="value">{s ? yuan(s.today_revenue_cents) : "—"}</div>
        </div>
      </div>
    </div>
  );
}
