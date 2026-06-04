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
        <h1>InstAtt 自动签到</h1>
        <p className="muted">用学校账号登录,即可挂机自动签到</p>
        <div className="spacer" />

        {err && <div className="banner bad">{err}</div>}

        {!device ? (
          <button style={{ width: "100%" }} onClick={startSchool}>
            用学校账号登录
          </button>
        ) : (
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
          </>
        )}
      </div>
    </div>
  );
}
