import { useEffect, useState } from "react";
import { get, yuan } from "../api";

interface Tx {
  id: number;
  amount_cents: number;
  type: string;
  balance_after: number;
  note: string;
  operator: string;
  created_at: string;
}

const TYPE_LABEL: Record<string, string> = {
  topup: "充值",
  sign_charge: "签到扣费",
  refund: "退款",
  adjust: "调整",
  register_bonus: "注册赠送",
};

export default function Transactions() {
  const [rows, setRows] = useState<Tx[]>([]);
  const [page, setPage] = useState(0);

  useEffect(() => {
    get<{ transactions: Tx[] }>(`/api/me/transactions?page=${page}`).then((d) =>
      setRows(d.transactions)
    );
  }, [page]);

  return (
    <div>
      <h2 className="page-title">余额流水</h2>
      <div className="card">
        <table>
          <thead>
            <tr>
              <th>时间</th>
              <th>类型</th>
              <th>金额</th>
              <th>余额</th>
              <th>备注</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((t) => (
              <tr key={t.id}>
                <td>{new Date(t.created_at).toLocaleString()}</td>
                <td>{TYPE_LABEL[t.type] || t.type}</td>
                <td style={{ color: t.amount_cents >= 0 ? "var(--good)" : "var(--bad)" }}>
                  {t.amount_cents >= 0 ? "+" : ""}
                  {yuan(t.amount_cents)}
                </td>
                <td>{yuan(t.balance_after)}</td>
                <td className="muted">{t.note}</td>
              </tr>
            ))}
            {!rows.length && (
              <tr>
                <td colSpan={5} className="muted center">
                  暂无流水
                </td>
              </tr>
            )}
          </tbody>
        </table>
        <div className="spacer" />
        <div className="row between">
          <button className="ghost" disabled={page === 0} onClick={() => setPage((p) => p - 1)}>
            上一页
          </button>
          <span className="muted">第 {page + 1} 页</span>
          <button className="ghost" disabled={rows.length < 20} onClick={() => setPage((p) => p + 1)}>
            下一页
          </button>
        </div>
      </div>
    </div>
  );
}
