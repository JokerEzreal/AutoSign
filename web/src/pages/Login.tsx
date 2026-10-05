import { useEffect, useRef, useState } from "react";
import { post, get } from "../api";
import { useMe } from "../auth";

interface DeviceStart {
  session_id: number;
  user_code: string;
  verification_uri: string;
  interval: number;
  expires_in: number;
}

export default function Login() {
  const { reload } = useMe();
  const [device, setDevice] = useState<DeviceStart | null>(null);
  const [status, setStatus] = useState<string>("");
  const [err, setErr] = useState<string>("");
  const timer = useRef<number | null>(null);

  // 次要入口:账号密码直登(多数账号强制 MFA 会失败,仅对未开启双重验证的账号可用)
  const [showPassword, setShowPassword] = useState(false);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);

  const startSchool = async () => {
    setErr("");
    setStatus("正在发起登录…");
    try {
      const d = await post<DeviceStart>("/api/auth/device/start");
      setDevice(d);
      setStatus("请在浏览器打开下方网址并输入代码完成授权…");
    } catch (e: any) {
      setErr(e.message);
      setStatus("");
    }
  };

  const loginPassword = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!username.trim() || !password) {
      setErr("请输入账号和密码");
      return;
    }
    setErr("");
    setBusy(true);
    setStatus("正在登录…");
    try {
      await post("/api/auth/password/login", { username: username.trim(), password });
      await reload();
    } catch (e: any) {
      setErr(e.message + "(若账号开启了双重验证,请改用「微软页面授权」)");
      setStatus("");
    } finally {
      setBusy(false);
    }
  };

  const stop = () => {
    if (timer.current) {
      clearInterval(timer.current);
      timer.current = null;
    }
  };

  useEffect(() => {
    if (!device) return;
    const poll = async () => {
      try {
        const r = await get<{ status: string }>(`/api/auth/device/poll?id=${device.session_id}`);
        if (r.status === "done") {
          stop();
          await reload();
        } else if (r.status === "expired") {
          stop();
          setErr("验证码已过期,请重试");
          setDevice(null);
        } else if (r.status === "declined") {
          stop();
          setErr("授权被拒绝");
          setDevice(null);
        }
      } catch {
        /* 网络抖动,继续轮询 */
      }
    };
    timer.current = window.setInterval(poll, (device.interval || 5) * 1000);
    return stop;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device]);

  return (
    <div className="login-wrap">
      <div className="card login-card">
        <h1>Instatt 自动签到</h1>
        <p className="muted">用学校账号登录,即可挂机自动签到</p>
        <div className="spacer" />

        {err && <div className="banner bad">{err}</div>}

        {device ? (
          <>
            <div className="code-box">{device.user_code}</div>
            <p className="center muted">
              打开{" "}
              <a href={device.verification_uri} target="_blank" rel="noreferrer">
                {device.verification_uri}
              </a>
              <br />
              输入上方代码并用学校账号授权
            </p>
            <p className="center">{status}</p>
            <div className="spacer" />
            <button className="ghost" style={{ width: "100%" }} onClick={() => (stop(), setDevice(null), setStatus(""))}>
              返回
            </button>
          </>
        ) : showPassword ? (
          <>
            <form onSubmit={loginPassword}>
              <div className="field">
                <label>学校账号</label>
                <input
                  placeholder="如 niubi666(可省略 @nottingham.edu.my)"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  autoFocus
                  disabled={busy}
                />
              </div>
              <div className="field">
                <label>密码</label>
                <input
                  type="password"
                  placeholder="学校账号密码"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  disabled={busy}
                />
              </div>
              <button style={{ width: "100%" }} type="submit" disabled={busy}>
                {busy ? "登录中…" : "登录"}
              </button>
            </form>
            {status && <p className="center muted" style={{ marginTop: 10 }}>{status}</p>}
            <div className="spacer" />
            <button className="ghost" style={{ width: "100%" }} onClick={() => (setShowPassword(false), setErr(""), setStatus(""))} disabled={busy}>
              返回
            </button>
          </>
        ) : (
          <>
            <button style={{ width: "100%" }} onClick={startSchool}>
              用学校账号登录
            </button>
            <div className="spacer" />
            <p className="center muted" style={{ fontSize: 12 }}>其他方式</p>
            <button className="ghost" style={{ width: "100%" }} onClick={() => (setShowPassword(true), setErr(""))}>
              用账号密码登录(需未开启双重验证)
            </button>
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
