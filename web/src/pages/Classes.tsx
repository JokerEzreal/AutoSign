import { useEffect, useState } from "react";
import { get } from "../api";
import { detailText } from "./Records";

interface SignInfo {
  result: string;
  detail: string;
  charged_cents: number;
  created_at: string;
}

interface ClassView {
  module_key: string;
  module_code: string;
  module_name: string;
  venue: string;
  venue_supported: boolean;
  class_type: number;
  class_type_label: string;
  start_time: number;
  end_time: number;
  class_status: number;
  attended: boolean;
  attendance_time: number;
  unlocked: boolean;
  unlock_user_name: string;
  students_attended: number;
  enabled: boolean;
  sign: SignInfo | null;
  status: string;
}

interface Resp {
  date: number;
  today: number;
  now_hhmm: number;
  classes: ClassView[];
}

const pad2 = (n: number) => String(n).padStart(2, "0");
/** 930 → "09:30" */
const hhmm = (t: number) => `${pad2(Math.floor(t / 100))}:${pad2(t % 100)}`;
/** 20260930 → Date(UTC 零点,仅用于日期运算) */
const toDate = (d: number) =>
  new Date(Date.UTC(Math.floor(d / 10000), (Math.floor(d / 100) % 100) - 1, d % 100));
const fromDate = (dt: Date) =>
  dt.getUTCFullYear() * 10000 + (dt.getUTCMonth() + 1) * 100 + dt.getUTCDate();
const shiftDate = (d: number, n: number) => {
  const dt = toDate(d);
  dt.setUTCDate(dt.getUTCDate() + n);
  return fromDate(dt);
};
const WEEKDAYS = ["日", "一", "二", "三", "四", "五", "六"];
const dateLabel = (d: number) =>
  `${Math.floor(d / 10000)}-${pad2(Math.floor(d / 100) % 100)}-${pad2(d % 100)} 周${WEEKDAYS[toDate(d).getUTCDay()]}`;
/** 202609300912 → "09:12" */
const attendedAt = (t: number) => (t ? hhmm(t % 10000) : "");

function statusBadge(c: ClassView) {
  switch (c.status) {
    case "attended":
      return <span className="badge good">已签到</span>;
    case "unlocked":
      return <span className="badge good">已解锁 · 签到中</span>;
    case "locked":
      return <span className="badge warn">上课中 · 未解锁</span>;
    case "upcoming":
      return <span className="badge muted">未开始</span>;
    case "absent":
      return <span className="badge bad">已上课 · 未签到</span>;
    case "not_unlocked":
      return <span className="badge muted">已结束 · 未解锁</span>;
    case "cancelled":
      return <span className="badge muted">已取消</span>;
    default:
      return <span className="badge muted">{c.status}</span>;
  }
}

/** 状态下方的补充说明 */
function statusNote(c: ClassView): string {
  if (c.status === "attended") return c.attendance_time ? `官方记录 ${attendedAt(c.attendance_time)}` : "官方已记录";
  if (c.status === "unlocked") {
    const who = c.unlock_user_name ? `${c.unlock_user_name} 解锁` : "";
    return [who, `${c.students_attended} 人已签`].filter(Boolean).join(" · ");
  }
  return "";
}

function signCell(c: ClassView) {
  if (c.sign) {
    if (c.sign.result === "ok_200") return <span className="badge good">已自动签到</span>;
    if (c.sign.result === "already_202") return <span className="badge muted">已签(重复)</span>;
    return (
      <>
        <span className="badge bad">失败</span>
        <div className="muted" style={{ fontSize: 12 }}>
          {detailText(c.sign.detail)}
        </div>
      </>
    );
  }
  if (!c.enabled) return <span className="muted">未启用</span>;
  if (!c.venue_supported) return <span className="muted">教室暂不支持</span>;
  if (c.status === "upcoming" || c.status === "locked" || c.status === "unlocked") {
    return <span className="muted">等待解锁后自动签</span>;
  }
  return <span className="muted">—</span>;
}

export default function Classes() {
  // null = 由后端决定「今天」(校区时区)
  const [date, setDate] = useState<number | null>(null);
  const [data, setData] = useState<Resp | null>(null);
  const [err, setErr] = useState("");
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const r = await get<Resp>("/api/me/classes" + (date ? `?date=${date}` : ""));
        if (alive) {
          setData(r);
          setErr("");
        }
      } catch (e: any) {
        if (alive) setErr(e.message);
      } finally {
        if (alive) setLoading(false);
      }
    };
    setLoading(true);
    load();
    // 解锁状态会变,定时刷新
    const timer = setInterval(load, 30000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [date]);

  const shown = data?.date ?? date;
  const isToday = !!data && data.date === data.today;
  const rows = data?.classes ?? [];
  const unlockedCount = rows.filter((c) => c.status === "unlocked").length;
  const attendedCount = rows.filter((c) => c.status === "attended").length;

  return (
    <div>
      <h2 className="page-title">课程表</h2>

      {err && <div className="banner bad">加载失败:{err}</div>}

      <div className="card">
        <div className="row between" style={{ flexWrap: "wrap", gap: 12 }}>
          <div className="row">
            <button className="ghost" disabled={!shown} onClick={() => shown && setDate(shiftDate(shown, -1))}>
              ‹ 前一天
            </button>
            <button className="ghost" disabled={isToday} onClick={() => setDate(null)}>
              今天
            </button>
            <button className="ghost" disabled={!shown} onClick={() => shown && setDate(shiftDate(shown, 1))}>
              后一天 ›
            </button>
          </div>
          <div style={{ textAlign: "right" }}>
            <div style={{ fontFamily: "var(--font-mono)", fontSize: 15, fontWeight: 600 }}>
              {shown ? dateLabel(shown) : "…"}
            </div>
            <div className="muted" style={{ fontSize: 12 }}>
              {data
                ? isToday
                  ? `校区时间 ${hhmm(data.now_hhmm)} · ${rows.length} 节课 · 解锁中 ${unlockedCount} · 已签到 ${attendedCount} · 每 30 秒刷新`
                  : `${rows.length} 节课 · 已签到 ${attendedCount}`
                : loading
                  ? "加载中…"
                  : ""}
            </div>
          </div>
        </div>
        <div className="spacer" />
        <table>
          <thead>
            <tr>
              <th>时间</th>
              <th>课程</th>
              <th>教室</th>
              <th>状态</th>
              <th>自动签到</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((c) => (
              <tr key={`${c.module_key}_${c.start_time}_${c.venue}`}>
                <td style={{ fontFamily: "var(--font-mono)", whiteSpace: "nowrap" }}>
                  {hhmm(c.start_time)}–{hhmm(c.end_time)}
                </td>
                <td>
                  {c.module_name || c.module_code}
                  <div className="muted" style={{ fontSize: 12 }}>
                    {c.module_code}
                    {c.class_type_label && ` · ${c.class_type_label}`}
                  </div>
                </td>
                <td>
                  <span style={{ fontFamily: "var(--font-mono)" }}>{c.venue}</span>
                  {!c.venue_supported && (
                    <div className="muted" style={{ fontSize: 12 }}>
                      暂不支持
                    </div>
                  )}
                </td>
                <td>
                  {statusBadge(c)}
                  {statusNote(c) && (
                    <div className="muted" style={{ fontSize: 12 }}>
                      {statusNote(c)}
                    </div>
                  )}
                </td>
                <td>{signCell(c)}</td>
              </tr>
            ))}
            {!rows.length && !loading && !err && (
              <tr>
                <td colSpan={5} className="muted center">
                  这一天没有课
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
