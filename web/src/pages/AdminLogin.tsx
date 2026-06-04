import { useState } from "react";
import { post } from "../api";
import { useMe } from "../auth";

// 独立的管理员登录页(用户登录页不暴露此入口)。
export default function AdminLogin() {
  const { reload } = useMe();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [err, setErr] = useState("");

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErr("");
    try {
      await post("/api/auth/admin/login", { username, password });
      await reload();
    } catch (e: any) {
      setErr(e.message);
    }
  };

  return (
    <div className="login-wrap">
      <div className="card login-card">
        <h1>管理后台</h1>
        <p className="muted">管理员登录</p>
        <div className="spacer" />
        {err && <div className="banner bad">{err}</div>}
        <form onSubmit={submit}>
          <div className="field">
            <label>账号</label>
            <input value={username} onChange={(e) => setUsername(e.target.value)} autoFocus />
          </div>
          <div className="field">
            <label>密码</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </div>
          <button style={{ width: "100%" }} type="submit">
            登录
          </button>
        </form>
      </div>
    </div>
  );
}
