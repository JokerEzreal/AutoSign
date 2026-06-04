import { useEffect, useState } from "react";
import { get, yuan } from "../api";

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

function resultBadge(r: string) {
  if (r === "ok_200") return <span className="badge good">成功</span>;
  if (r === "already_202") return <span className="badge muted">已签</span>;
  return <span className="badge bad">失败</span>;
}

/** 把内部 detail 翻译成用户可读说明 */
function detailText(detail: string): string {
  if (detail.startsWith("missing_bssid:")) {
    return "暂不支持该教室 " + detail.slice("missing_bssid:".length);
  }
  if (detail === "missing_device_uid") return "设备未注册,请重新登录";
  if (detail.startsWith("status_")) return "签到被拒:" + detail;
  return detail;
}

export default function Records() {
  const [rows, setRows] = useState<Rec[]>([]);
  const [page, setPage] = useState(0);

  useEffect(() => {
    get<{ records: Rec[] }>(`/api/me/records?page=${page}`).then((d) => setRows(d.records));
  }, [page]);

  return (
    <div>
      <h2 className="page-title">签到记录</h2>
      <div className="card">
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
            {rows.map((r) => (
              <tr key={r.id}>
                <td>{new Date(r.created_at).toLocaleString()}</td>
                <td>
                  {r.module_name || r.module_key}
                  <div className="muted" style={{ fontSize: 12 }}>
                    {r.module_key}
                  </div>
                </td>
                <td>{r.venue}</td>
                <td>
                  {resultBadge(r.result)}
                  {r.result === "failed" && r.detail && (
                    <div className="muted" style={{ fontSize: 12 }}>
                      {detailText(r.detail)}
                    </div>
                  )}
                </td>
                <td>{r.charged_cents ? yuan(r.charged_cents) : "—"}</td>
              </tr>
            ))}
            {!rows.length && (
              <tr>
                <td colSpan={5} className="muted center">
                  暂无记录
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
    </div>
  );
}
