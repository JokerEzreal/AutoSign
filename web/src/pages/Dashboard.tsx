import { useState } from "react";
import { patch, post, yuan } from "../api";
import { useMe } from "../auth";

export default function Dashboard() {
  const { me, reload } = useMe();
  const acc = me?.account;
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  if (!acc) return null;

  const toggleAuto = async () => {
    setBusy(true);
    try {
      await post("/api/me/auto-sign", { auto_sign: !acc.auto_sign });
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const toggleModule = async (mod: string, on: boolean) => {
    const next = on
      ? [...acc.enabled_modules, mod]
      : acc.enabled_modules.filter((m) => m !== mod);
    setBusy(true);
    try {
      await patch("/api/me/modules", { enabled: next });
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const sync = async () => {
    setBusy(true);
    setMsg("");
    try {
      const r = await post<{ modules: string[] }>("/api/me/sync");
      setMsg(`同步完成,课程 ${r.modules.length} 门`);
      await reload();
    } catch (e: any) {
      setMsg("同步失败:" + e.message);
    } finally {
      setBusy(false);
    }
  };

  const statusBadge =
    acc.status === "active" ? (
      <span className="badge good">正常</span>
    ) : acc.status === "needs_relogin" ? (
      <span className="badge bad">需重新登录</span>
    ) : (
      <span className="badge muted">已停用</span>
    );

  return (
    <div>
      <h2 className="page-title">仪表盘</h2>

      {acc.status === "needs_relogin" && (
        <div className="banner warn">
          登录已失效,自动签到已暂停。请退出后用学校账号重新登录。
        </div>
      )}

      <div className="grid">
        <div className="stat">
          <div className="label">账户余额</div>
          <div className="value">{yuan(acc.balance_cents)}</div>
        </div>
        <div className="stat">
          <div className="label">挂机状态</div>
          <div className="value" style={{ fontSize: 18 }}>
            {acc.auto_sign ? (
              <span className="badge good">已开启</span>
            ) : (
              <span className="badge muted">已关闭</span>
            )}
          </div>
        </div>
        <div className="stat">
          <div className="label">账号状态</div>
          <div className="value" style={{ fontSize: 18 }}>
            {statusBadge}
          </div>
        </div>
      </div>

      <div className="card">
        <h3>账号信息</h3>
        <div className="kv">
          <span className="muted">学校账号</span>
          <span>{acc.account_name}</span>
        </div>
        <div className="kv">
          <span className="muted">学号</span>
          <span>{acc.student_id || "—"}</span>
        </div>
        <div className="kv">
          <span className="muted">专业</span>
          <span>{acc.course || "—"}</span>
        </div>
        <div className="kv">
          <span className="muted">上次同步</span>
          <span>{acc.last_synced_at ? new Date(acc.last_synced_at).toLocaleString() : "—"}</span>
        </div>
      </div>

      <div className="card">
        <div className="row between">
          <div>
            <h3 style={{ margin: 0 }}>课程自动签到({acc.enabled_modules.length}/{acc.modules.length})</h3>
            <p className="muted" style={{ margin: "6px 0 0" }}>
              勾选要自动签到的课程,未勾选的不会自动签。
            </p>
          </div>
          <button className="ghost" onClick={sync} disabled={busy}>
            {busy ? "处理中…" : "立即同步"}
          </button>
        </div>
        <div className="spacer" />
        {acc.modules.length ? (
          <div className="course-list">
            {acc.modules.map((m) => {
              const on = acc.enabled_modules.includes(m);
              return (
                <label key={m} className={"course-item" + (on ? " on" : "")}>
                  <input
                    type="checkbox"
                    checked={on}
                    disabled={busy}
                    onChange={() => toggleModule(m, !on)}
                  />
                  <span>{m}</span>
                </label>
              );
            })}
          </div>
        ) : (
          <p className="muted">暂无课程,点击「立即同步」从学校系统拉取。</p>
        )}
        {msg && <div className="error-text" style={{ color: "var(--muted)" }}>{msg}</div>}
      </div>

      <div className="card">
        <div className="row between">
          <div>
            <h3 style={{ margin: 0 }}>自动签到</h3>
            <p className="muted" style={{ margin: "6px 0 0" }}>
              开启后,系统会在你的课程解锁时自动签到并按成功次数计费。
            </p>
          </div>
          <button onClick={toggleAuto} disabled={busy} className={acc.auto_sign ? "danger" : ""}>
            {acc.auto_sign ? "关闭挂机" : "开启挂机"}
          </button>
        </div>
      </div>
    </div>
  );
}
