import { useEffect, useState } from "react";
import { get, patch, post } from "../../api";

const FIELDS: { key: string; label: string; hint: string }[] = [
  { key: "price_per_sign_cents", label: "每次签到单价(分)", hint: "成功签到一次扣除的金额,单位分" },
  { key: "poll_interval_sec", label: "轮询间隔(秒)", hint: "多久检查一次解锁课程" },
  { key: "max_concurrency", label: "最大并发", hint: "同时发起签到请求的上限" },
  { key: "jitter_ms_max", label: "抖动上限(毫秒)", hint: "每次签到前的随机延迟上限" },
];

export default function Settings() {
  const [cfg, setCfg] = useState<Record<string, string>>({});
  const [msg, setMsg] = useState("");

  useEffect(() => {
    get<{ config: Record<string, string> }>("/api/admin/config").then((d) => setCfg(d.config));
  }, []);

  const saveConfig = async () => {
    setMsg("");
    try {
      const r = await patch<{ note: string }>("/api/admin/config", cfg);
      setMsg(r.note || "已保存");
    } catch (e: any) {
      setMsg("保存失败:" + e.message);
    }
  };

  // 改密码
  const [oldp, setOldp] = useState("");
  const [newp, setNewp] = useState("");
  const [pmsg, setPmsg] = useState("");
  const changePwd = async () => {
    setPmsg("");
    try {
      await post("/api/admin/superadmin/password", { old: oldp, new: newp });
      setPmsg("密码已修改");
      setOldp("");
      setNewp("");
    } catch (e: any) {
      setPmsg("失败:" + e.message);
    }
  };

  return (
    <div>
      <h2 className="page-title">系统配置</h2>

      <div className="card">
        <h3>引擎参数</h3>
        {FIELDS.map((f) => (
          <div className="field" key={f.key}>
            <label>
              {f.label} <span className="muted">— {f.hint}</span>
            </label>
            <input
              value={cfg[f.key] ?? ""}
              onChange={(e) => setCfg({ ...cfg, [f.key]: e.target.value })}
            />
          </div>
        ))}
        <div className="row between">
          <span className="muted">{msg}</span>
          <button onClick={saveConfig}>保存配置</button>
        </div>
      </div>

      <div className="card">
        <h3>修改超管密码</h3>
        <div className="field">
          <label>原密码</label>
          <input type="password" value={oldp} onChange={(e) => setOldp(e.target.value)} />
        </div>
        <div className="field">
          <label>新密码(至少 6 位)</label>
          <input type="password" value={newp} onChange={(e) => setNewp(e.target.value)} />
        </div>
        <div className="row between">
          <span className="muted">{pmsg}</span>
          <button onClick={changePwd}>修改密码</button>
        </div>
      </div>
    </div>
  );
}
