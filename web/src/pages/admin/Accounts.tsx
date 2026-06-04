import { useEffect, useState } from "react";
import { get, patch, post, yuan } from "../../api";
import { useMe } from "../../auth";

interface Acc {
  id: number;
  account_name: string;
  student_id: string;
  balance_cents: number;
  auto_sign: boolean;
  role: string;
  status: string;
  created_at: string;
}

export default function Accounts() {
  const { me } = useMe();
  const isSuper = me?.role === "superadmin";
  const [rows, setRows] = useState<Acc[]>([]);
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const [topupFor, setTopupFor] = useState<Acc | null>(null);

  const load = () => {
    get<{ accounts: Acc[] }>(`/api/admin/accounts?search=${encodeURIComponent(search)}&page=${page}`).then(
      (d) => setRows(d.accounts)
    );
  };
  useEffect(load, [page]);

  const setStatus = async (a: Acc, status: string) => {
    await patch(`/api/admin/accounts/${a.id}/status`, { status });
    load();
  };
  const setRole = async (a: Acc, role: string) => {
    await patch(`/api/admin/accounts/${a.id}/role`, { role });
    load();
  };

  return (
    <div>
      <h2 className="page-title">账号管理</h2>
      <div className="card">
        <div className="row" style={{ marginBottom: 14 }}>
          <input
            placeholder="搜索学校账号…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && (setPage(0), load())}
            style={{ maxWidth: 280 }}
          />
          <button className="ghost" onClick={() => (setPage(0), load())}>
            搜索
          </button>
        </div>
        <table>
          <thead>
            <tr>
              <th>学校账号</th>
              <th>学号</th>
              <th>余额</th>
              <th>挂机</th>
              <th>角色</th>
              <th>状态</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((a) => (
              <tr key={a.id}>
                <td>{a.account_name}</td>
                <td className="muted">{a.student_id}</td>
                <td>{yuan(a.balance_cents)}</td>
                <td>{a.auto_sign ? "开" : "关"}</td>
                <td>
                  {isSuper ? (
                    <select
                      value={a.role}
                      onChange={(e) => setRole(a, e.target.value)}
                      style={{ width: 90 }}
                    >
                      <option value="user">user</option>
                      <option value="admin">admin</option>
                    </select>
                  ) : (
                    a.role
                  )}
                </td>
                <td>
                  {a.status === "active" ? (
                    <span className="badge good">正常</span>
                  ) : a.status === "needs_relogin" ? (
                    <span className="badge warn">需登录</span>
                  ) : (
                    <span className="badge muted">停用</span>
                  )}
                </td>
                <td>
                  <div className="row" style={{ gap: 6 }}>
                    <button className="ghost" onClick={() => setTopupFor(a)}>
                      充值
                    </button>
                    {a.status === "disabled" ? (
                      <button className="ghost" onClick={() => setStatus(a, "active")}>
                        启用
                      </button>
                    ) : (
                      <button className="ghost" onClick={() => setStatus(a, "disabled")}>
                        停用
                      </button>
                    )}
                  </div>
                </td>
              </tr>
            ))}
            {!rows.length && (
              <tr>
                <td colSpan={7} className="muted center">
                  无账号
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

      {topupFor && (
        <TopupModal
          acc={topupFor}
          onClose={() => setTopupFor(null)}
          onDone={() => {
            setTopupFor(null);
            load();
          }}
        />
      )}
    </div>
  );
}

function TopupModal({ acc, onClose, onDone }: { acc: Acc; onClose: () => void; onDone: () => void }) {
  const [yuanStr, setYuanStr] = useState("");
  const [note, setNote] = useState("");
  const [err, setErr] = useState("");

  const submit = async () => {
    setErr("");
    const amount = Math.round(parseFloat(yuanStr) * 100);
    if (!amount || isNaN(amount)) {
      setErr("请输入有效金额");
      return;
    }
    try {
      await post(`/api/admin/accounts/${acc.id}/topup`, { amount_cents: amount, note });
      onDone();
    } catch (e: any) {
      setErr(e.message);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="card modal" onClick={(e) => e.stopPropagation()}>
        <h3>为 {acc.account_name} 充值</h3>
        <p className="muted">当前余额 {yuan(acc.balance_cents)}</p>
        <div className="field">
          <label>金额(元,可负数用于扣减)</label>
          <input value={yuanStr} onChange={(e) => setYuanStr(e.target.value)} autoFocus />
        </div>
        <div className="field">
          <label>备注</label>
          <input value={note} onChange={(e) => setNote(e.target.value)} />
        </div>
        {err && <div className="error-text">{err}</div>}
        <div className="row between" style={{ marginTop: 10 }}>
          <button className="ghost" onClick={onClose}>
            取消
          </button>
          <button onClick={submit}>确认充值</button>
        </div>
      </div>
    </div>
  );
}
