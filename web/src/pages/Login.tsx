import { useEffect, useRef, useState } from "react";
import { post, get } from "../api";
import { useMe } from "../auth";

type Phase = "form" | "working";

export default function Login() {
  const { reload } = useMe();
  const [phase, setPhase] = useState<Phase>("form");
  const [err, setErr] = useState("");
  const [status, setStatus] = useState("");

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [mfaNumber, setMfaNumber] = useState<string | null>(null);
  const sessionId = useRef<string | null>(null);
  const timer = useRef<number | null>(null);
  const deadline = useRef<number>(0);

  const stop = () => {
    if (timer.current) { clearInterval(timer.current); timer.current = null; }
  };
  useEffect(() => stop, []);

  const startLogin = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!username.trim()) { setErr("请输入学校账号"); return; }
    setErr(""); setMfaNumber(null); setStatus("正在登录,请稍候…"); setPhase("working");
    try {
      const r = await post<{ session_id: string }>("/api/auth/assisted/start", {
        username: username.trim(), password,
      });
      sessionId.current = r.session_id;
      deadline.current = Date.now() + 200_000;
      timer.current = window.setInterval(poll, 3000);
    } catch (e: any) {
      setErr(e.message); setPhase("form"); setStatus("");
    }
  };

  const poll = async () => {
    if (!sessionId.current) return;
    if (Date.now() > deadline.current) {
      stop(); setErr("登录超时,请重试"); setPhase("form"); setStatus(""); return;
    }
    try {
      const r = await get<{ status: string; number?: string; error?: string }>(
        `/api/auth/assisted/poll?id=${sessionId.current}`
      );
      if (r.status === "mfa") {
        setMfaNumber(r.number || null); setStatus("");
      } else if (r.status === "done") {
        stop(); await reload();
      } else if (r.status === "failed") {
        stop(); setErr(r.error || "登录失败"); setPhase("form"); setStatus("");
      }
    } catch {
      /* 网络抖动,继续轮询 */
    }
  };

  const cancel = () => { stop(); sessionId.current = null; setMfaNumber(null); setErr(""); setStatus(""); setPhase("form"); };

  return (
    <div className="login-wrap">
      <div className="card login-card">
        <h1>Instatt 自动签到</h1>
        <p className="muted">用学校账号登录,即可挂机自动签到</p>
        <div className="spacer" />

        {err && <div className="banner bad">{err}</div>}

        {phase === "form" && (
          <form onSubmit={startLogin}>
            <div className="field">
              <label>学校账号</label>
              <input
                placeholder="如 niubi666(可省略 @nottingham.edu.my)"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoFocus
              />
            </div>
            <div className="field">
              <label>密码</label>
              <input
                type="password"
                placeholder="学校账号密码"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </div>
            <button style={{ width: "100%" }} type="submit">登录</button>
            <p className="center muted" style={{ fontSize: 12, marginTop: 12 }}>
              登录后需在 Microsoft Authenticator 中批准并输入页面显示的数字
            </p>
          </form>
        )}

        {phase === "working" && (
          <>
            {mfaNumber ? (
              <>
                <p className="center muted">打开 Microsoft Authenticator,批准登录请求并输入数字:</p>
                <div className="code-box">{mfaNumber}</div>
                <p className="center muted" style={{ fontSize: 13 }}>批准后会自动登录,请稍候…</p>
              </>
            ) : (
              <p className="center">{status || "正在登录,请稍候…"}</p>
            )}
            <div className="spacer" />
            <button className="ghost" style={{ width: "100%" }} onClick={cancel}>取消</button>
          </>
        )}

        <div className="spacer" />
        <p className="center muted" style={{ fontSize: 12 }}>
          本项目目前已开源 ·{" "}
          <a href="https://github.com/JokerEzreal/AutoSign" target="_blank" rel="noopener noreferrer">
            GitHub
          </a>
        </p>
      </div>
    </div>
  );
}
