// 后端 API 封装(带 cookie)。

async function api<T = any>(path: string, opts: RequestInit = {}): Promise<T> {
  const res = await fetch(path, {
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    ...opts,
  });
  let data: any = {};
  try {
    data = await res.json();
  } catch {
    /* 空响应 */
  }
  if (!res.ok) {
    throw new Error(data?.error || `HTTP ${res.status}`);
  }
  return data as T;
}

export const get = <T = any>(p: string) => api<T>(p);
export const post = <T = any>(p: string, body?: any) =>
  api<T>(p, { method: "POST", body: body ? JSON.stringify(body) : undefined });
export const patch = <T = any>(p: string, body?: any) =>
  api<T>(p, { method: "PATCH", body: body ? JSON.stringify(body) : undefined });

/** 分 → 人民币展示 */
export const yuan = (cents: number) => "¥" + (cents / 100).toFixed(2);
