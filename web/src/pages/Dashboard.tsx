import { useEffect, useState } from "react";
import { get, patch, post, yuan } from "../api";
import { useMe } from "../auth";

interface Venues {
  bssid_venues: string[];
  ignore_wifi: string[];
}

export default function Dashboard() {
  const { me, reload } = useMe();
  const acc = me?.account;

  const [sel, setSel] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [venues, setVenues] = useState<Venues | null>(null);

  useEffect(() => {
    get<Venues>("/api/venues").then(setVenues).catch(() => {});
  }, []);

  // 账号数据变化时重置勾选
  useEffect(() => {
    if (acc) setSel(acc.enabled_modules);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [acc?.enabled_modules.join(",")]);

  if (!acc) return null;

  const dirty =
    JSON.stringify([...sel].sort()) !== JSON.stringify([...acc.enabled_modules].sort());

  const toggle = (m: string) =>
    setSel((s) => (s.includes(m) ? s.filter((x) => x !== m) : [...s, m]));

  const save = async () => {
    setBusy(true);
    setMsg("");
    try {
      await patch("/api/me/modules", { enabled: sel });
      await reload();
      setMsg("已保存,所选课程将自动签到");
    } catch (e: any) {
      setMsg("保存失败:" + e.message);
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

  const lowBalance = acc.balance_cents <= 0;

  return (
    <div>
      <h2 className="page-title">仪表盘</h2>

      {acc.status === "needs_relogin" && (
        <div className="banner warn">
          登录已失效,自动签到已暂停。请退出后用学校账号重新登录。
        </div>
      )}
      {lowBalance && (
        <div className="banner warn">余额不足,暂不会自动签到。请联系管理员充值。</div>
      )}

      <div className="grid">
        <div className="stat">
          <div className="label">账户余额</div>
          <div className="value">{yuan(acc.balance_cents)}</div>
        </div>
        <div className="stat">
          <div className="label">自动签到课程</div>
          <div className="value">
            {acc.enabled_modules.length}
            <span style={{ fontSize: 15, color: "var(--muted)" }}> / {acc.modules.length} 门</span>
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
            <h3 style={{ margin: 0 }}>课程自动签到</h3>
            <p className="muted" style={{ margin: "6px 0 0" }}>
              勾选要自动签到的课程后点「保存」即生效;课程解锁时自动签到,成功一次扣 ¥1.00,余额不足则不签。
            </p>
          </div>
          <button className="ghost" onClick={sync} disabled={busy}>
            {busy ? "处理中…" : "立即同步"}
          </button>
        </div>
        <div className="spacer" />
        {acc.modules.length ? (
          <>
            <div className="course-list">
              {acc.modules.map((m) => {
                const on = sel.includes(m);
                const name = acc.module_info[m];
                return (
                  <label key={m} className={"course-item" + (on ? " on" : "")}>
                    <input type="checkbox" checked={on} disabled={busy} onChange={() => toggle(m)} />
                    <div className="course-text">
                      <div className="course-id">{m}</div>
                      {name && <div className="course-name">{name}</div>}
                    </div>
                  </label>
                );
              })}
            </div>
            <div className="spacer" />
            <div className="row between">
              <span className="muted">{msg}</span>
              <button onClick={save} disabled={busy || !dirty}>
                {dirty ? "保存" : "已保存"}
              </button>
            </div>
          </>
        ) : (
          <p className="muted">暂无课程,点击「立即同步」从学校系统拉取。</p>
        )}
      </div>

      {venues && (
        <div className="card">
          <h3>支持的教室</h3>
          <p className="muted" style={{ margin: "0 0 14px" }}>
            只有在以下教室上课才能自动签到;其它教室暂不支持(签到会自动跳过并在记录中标注)。
          </p>
          <div className="row" style={{ flexWrap: "wrap", gap: 8 }}>
            {venues.bssid_venues.map((v) => (
              <span key={v} className="badge muted" style={{ fontFamily: "var(--font-mono)" }}>
                {v}
              </span>
            ))}
            {venues.ignore_wifi.map((v) => (
              <span
                key={v}
                className="badge good"
                style={{ fontFamily: "var(--font-mono)" }}
                title="免 WiFi 教室"
              >
                {v}
              </span>
            ))}
            <span className="badge pending">其他教室待添加…</span>
          </div>
        </div>
      )}
    </div>
  );
}
