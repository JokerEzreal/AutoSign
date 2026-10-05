import { useEffect, useState } from "react";
import { get, patch, yuan } from "../../api";
import { ClassView, hhmm, shiftDate, dateLabel, statusBadge, statusNote } from "../Classes";
import { resultBadge, detailText } from "../Records";

interface Detail {
  id: number;
  account_name: string;
  student_id: string;
  course: string;
  balance_cents: number;
  auto_sign: boolean;
  role: string;
  status: string;
  modules: string[];
  enabled_modules: string[];
  module_info: Record<string, string>;
  last_synced_at: string | null;
}

interface ClassesResp {
  date: number;
  today: number;
  now_hhmm: number;
  classes: ClassView[];
}

interface Rec {
  id: number;
  module_key: string;
  module_name: string;
  venue: string;
  class_date: number;
  start_time: number;
  result: string;
  charged_cents: number;
  detail: string;
  created_at: string;
}

/** 管理员查看/编辑某账号:资料 + 可编辑课程勾选 + 挂机开关 + 当天课表 + 签到流水。 */
export default function AccountDetail({
  id,
  name,
  onClose,
}: {
  id: number;
  name: string;
  onClose: () => void;
}) {
  const [detail, setDetail] = useState<Detail | null>(null);
  const [detailErr, setDetailErr] = useState("");
  const [date, setDate] = useState<number | null>(null); // null = 后端决定「今天」
  const [cls, setCls] = useState<ClassesResp | null>(null);
  const [clsErr, setClsErr] = useState("");
  const [clsLoading, setClsLoading] = useState(true);

  // 可编辑状态
  const [sel, setSel] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [saveMsg, setSaveMsg] = useState("");

  // 签到流水
  const [recs, setRecs] = useState<Rec[]>([]);
  const [recPage, setRecPage] = useState(0);
  const [recErr, setRecErr] = useState("");

  const loadDetail = () =>
    get<Detail>(`/api/admin/accounts/${id}/detail`)
      .then((d) => {
        setDetail(d);
        setSel(d.enabled_modules);
      })
      .catch((e) => setDetailErr(e.message));

  useEffect(() => {
    loadDetail();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  useEffect(() => {
    let alive = true;
    setClsLoading(true);
    get<ClassesResp>(`/api/admin/accounts/${id}/classes` + (date ? `?date=${date}` : ""))
      .then((r) => alive && (setCls(r), setClsErr("")))
      .catch((e) => alive && setClsErr(e.message))
      .finally(() => alive && setClsLoading(false));
    return () => {
      alive = false;
    };
  }, [id, date]);

  useEffect(() => {
    get<{ records: Rec[] }>(`/api/admin/accounts/${id}/records?page=${recPage}`)
      .then((r) => (setRecs(r.records), setRecErr("")))
      .catch((e) => setRecErr(e.message));
  }, [id, recPage]);

  const shown = cls?.date ?? date;
  const isToday = !!cls && cls.date === cls.today;
  const rows = cls?.classes ?? [];

  const venueByCourse = new Map<string, Set<string>>();
  for (const c of rows) {
    if (!venueByCourse.has(c.module_code)) venueByCourse.set(c.module_code, new Set());
    if (c.venue) venueByCourse.get(c.module_code)!.add(c.venue);
  }

  const dirty =
    !!detail &&
    JSON.stringify([...sel].sort()) !== JSON.stringify([...detail.enabled_modules].sort());

  const toggle = (m: string) =>
    setSel((s) => (s.includes(m) ? s.filter((x) => x !== m) : [...s, m]));

  const saveModules = async () => {
    setBusy(true);
    setSaveMsg("");
    try {
      await patch(`/api/admin/accounts/${id}/modules`, { enabled: sel });
      await loadDetail();
      setSaveMsg("已保存");
    } catch (e: any) {
      setSaveMsg("保存失败:" + e.message);
    } finally {
      setBusy(false);
    }
  };

  const toggleAutoSign = async () => {
    if (!detail) return;
    setBusy(true);
    setSaveMsg("");
    try {
      await patch(`/api/admin/accounts/${id}/auto-sign`, { auto_sign: !detail.auto_sign });
      await loadDetail();
    } catch (e: any) {
      setSaveMsg("挂机开关保存失败:" + e.message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="card"
        onClick={(e) => e.stopPropagation()}
        style={{ width: 820, maxWidth: "94vw", maxHeight: "88vh", overflow: "auto" }}
      >
        <div className="row between">
          <h3 style={{ margin: 0 }}>账号详情 · {name}</h3>
          <button className="ghost" onClick={onClose}>
            关闭
          </button>
        </div>

        {detailErr && <div className="banner bad" style={{ marginTop: 12 }}>资料加载失败:{detailErr}</div>}

        {detail && (
          <>
            <div className="spacer" />
            <div className="grid" style={{ gridTemplateColumns: "repeat(auto-fit,minmax(150px,1fr))", gap: 10 }}>
              <div className="kv"><span className="muted">学号</span><span>{detail.student_id || "—"}</span></div>
              <div className="kv"><span className="muted">专业</span><span>{detail.course || "—"}</span></div>
              <div className="kv"><span className="muted">余额</span><span>{yuan(detail.balance_cents)}</span></div>
              <div className="kv"><span className="muted">角色</span><span>{detail.role}</span></div>
              <div className="kv">
                <span className="muted">挂机</span>
                <span className="row" style={{ gap: 8, alignItems: "center" }}>
                  <span className={"badge " + (detail.auto_sign ? "good" : "muted")}>
                    {detail.auto_sign ? "开" : "关"}
                  </span>
                  <button className="ghost" style={{ padding: "2px 8px" }} disabled={busy} onClick={toggleAutoSign}>
                    {detail.auto_sign ? "关闭" : "开启"}
                  </button>
                </span>
              </div>
              <div className="kv"><span className="muted">状态</span><span>{detail.status}</span></div>
              <div className="kv">
                <span className="muted">上次同步</span>
                <span>{detail.last_synced_at ? new Date(detail.last_synced_at).toLocaleString() : "—"}</span>
              </div>
            </div>

            <div className="spacer" />
            <div className="row between" style={{ flexWrap: "wrap", gap: 8 }}>
              <h3 style={{ margin: "6px 0 10px", fontSize: 14 }}>
                课程(勾选 = 开启自动签,{sel.length}/{detail.modules.length} 门)
              </h3>
              <div className="row" style={{ gap: 8, alignItems: "center" }}>
                {saveMsg && <span className="muted" style={{ fontSize: 12 }}>{saveMsg}</span>}
                <button disabled={busy || !dirty} onClick={saveModules}>
                  保存课程勾选
                </button>
              </div>
            </div>
            {detail.modules.length ? (
              <div className="course-list">
                {detail.modules.map((m) => {
                  const on = sel.includes(m);
                  const venues = [...(venueByCourse.get(m) ?? [])];
                  return (
                    <label key={m} className={"course-item" + (on ? " on" : "")}>
                      <input type="checkbox" checked={on} disabled={busy} onChange={() => toggle(m)} />
                      <div className="course-text">
                        <div className="course-id">{m}</div>
                        {detail.module_info[m] && <div className="course-name">{detail.module_info[m]}</div>}
                        {venues.length > 0 && (
                          <div className="course-name" style={{ color: "var(--accent-2)" }}>
                            今日教室:{venues.join("、")}
                          </div>
                        )}
                      </div>
                    </label>
                  );
                })}
              </div>
            ) : (
              <p className="muted">暂无课程(该账号可能从未成功同步)。</p>
            )}
          </>
        )}

        <div className="spacer" />
        <div className="row between" style={{ flexWrap: "wrap", gap: 10 }}>
          <h3 style={{ margin: 0, fontSize: 14 }}>当天课表(课程 · 教室 · 状态)</h3>
          <div className="row" style={{ gap: 6 }}>
            <button className="ghost" disabled={!shown} onClick={() => shown && setDate(shiftDate(shown, -1))}>‹ 前一天</button>
            <button className="ghost" disabled={isToday} onClick={() => setDate(null)}>今天</button>
            <button className="ghost" disabled={!shown} onClick={() => shown && setDate(shiftDate(shown, 1))}>后一天 ›</button>
          </div>
        </div>
        <div className="muted" style={{ fontSize: 12, margin: "6px 0" }}>
          {shown ? dateLabel(shown) : "…"}
          {cls && isToday && ` · 校区时间 ${hhmm(cls.now_hhmm)}`}
        </div>

        {clsErr && <div className="banner bad">课表加载失败:{clsErr}</div>}

        <table>
          <thead>
            <tr>
              <th>时间</th>
              <th>课程</th>
              <th>教室</th>
              <th>状态</th>
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
                  {!c.venue_supported && <div className="muted" style={{ fontSize: 12 }}>暂不支持</div>}
                </td>
                <td>
                  {statusBadge(c)}
                  {statusNote(c) && <div className="muted" style={{ fontSize: 12 }}>{statusNote(c)}</div>}
                </td>
              </tr>
            ))}
            {!rows.length && !clsLoading && !clsErr && (
              <tr>
                <td colSpan={4} className="muted center">这一天没有课</td>
              </tr>
            )}
          </tbody>
        </table>

        <div className="spacer" />
        <h3 style={{ margin: "6px 0 10px", fontSize: 14 }}>签到流水</h3>
        {recErr && <div className="banner bad">流水加载失败:{recErr}</div>}
        <table>
          <thead>
            <tr>
              <th>时间</th>
              <th>课程</th>
              <th>教室</th>
              <th>结果</th>
              <th>扣费</th>
            </tr>
          </thead>
          <tbody>
            {recs.map((r) => (
              <tr key={r.id}>
                <td style={{ whiteSpace: "nowrap" }}>{new Date(r.created_at).toLocaleString()}</td>
                <td>
                  {r.module_name || r.module_key}
                  <div className="muted" style={{ fontSize: 12 }}>{r.module_key}</div>
                </td>
                <td>{r.venue}</td>
                <td>
                  {resultBadge(r.result)}
                  {r.result === "failed" && r.detail && (
                    <div className="muted" style={{ fontSize: 12 }}>{detailText(r.detail)}</div>
                  )}
                </td>
                <td>{r.charged_cents ? yuan(r.charged_cents) : "—"}</td>
              </tr>
            ))}
            {!recs.length && (
              <tr>
                <td colSpan={5} className="muted center">暂无签到流水</td>
              </tr>
            )}
          </tbody>
        </table>
        <div className="spacer" />
        <div className="row between">
          <button className="ghost" disabled={recPage === 0} onClick={() => setRecPage((p) => p - 1)}>
            上一页
          </button>
          <span className="muted">第 {recPage + 1} 页</span>
          <button className="ghost" disabled={recs.length < 20} onClick={() => setRecPage((p) => p + 1)}>
            下一页
          </button>
        </div>
      </div>
    </div>
  );
}
