import { Navigate, NavLink, Route, Routes, useNavigate } from "react-router-dom";
import { useMe } from "./auth";
import { post } from "./api";
import Login from "./pages/Login";
import AdminLogin from "./pages/AdminLogin";
import Dashboard from "./pages/Dashboard";
import Records from "./pages/Records";
import Transactions from "./pages/Transactions";
import Accounts from "./pages/admin/Accounts";
import Stats from "./pages/admin/Stats";
import Settings from "./pages/admin/Settings";

function Layout({ children }: { children: React.ReactNode }) {
  const { me, reload } = useMe();
  const nav = useNavigate();
  const isAdmin = me?.role === "admin" || me?.role === "superadmin";
  const isSuper = me?.role === "superadmin";
  const isUser = !!me?.account;

  const logout = async () => {
    await post("/api/auth/logout");
    await reload();
    nav("/login");
  };

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <span className="mark">
            <span className="dot" />
            InstAtt
          </span>
          <small>自动签到面板</small>
        </div>
        {isUser && (
          <>
            <NavLink to="/" end className="navlink">
              仪表盘
            </NavLink>
            <NavLink to="/records" className="navlink">
              签到记录
            </NavLink>
            <NavLink to="/transactions" className="navlink">
              余额流水
            </NavLink>
          </>
        )}
        {isAdmin && (
          <>
            <div className="nav-section">管理</div>
            <NavLink to="/admin/stats" className="navlink">
              统计看板
            </NavLink>
            <NavLink to="/admin/accounts" className="navlink">
              账号管理
            </NavLink>
            {isSuper && (
              <NavLink to="/admin/settings" className="navlink">
                系统配置
              </NavLink>
            )}
          </>
        )}
        <div className="nav-spacer" />
        <div className="nav-user">{me?.account ? me.account.account_name : me?.role}</div>
        <button className="ghost" onClick={logout}>
          退出登录
        </button>
      </aside>
      <main className="content">
        <div className="content-inner">{children}</div>
      </main>
    </div>
  );
}

export default function App() {
  const { me, loading } = useMe();

  if (loading) {
    return <div className="login-wrap muted">加载中…</div>;
  }

  if (!me) {
    return (
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="/manage" element={<AdminLogin />} />
        <Route path="*" element={<Navigate to="/login" replace />} />
      </Routes>
    );
  }

  const isAdmin = me.role === "admin" || me.role === "superadmin";
  const isSuper = me.role === "superadmin";
  const hasAccount = !!me.account;
  // 超管无个人账号 → 默认进管理看板
  const home = hasAccount ? "/" : "/admin/stats";

  return (
    <Layout>
      <Routes>
        {hasAccount && <Route path="/" element={<Dashboard />} />}
        {hasAccount && <Route path="/records" element={<Records />} />}
        {hasAccount && <Route path="/transactions" element={<Transactions />} />}
        {isAdmin && <Route path="/admin/stats" element={<Stats />} />}
        {isAdmin && <Route path="/admin/accounts" element={<Accounts />} />}
        {isSuper && <Route path="/admin/settings" element={<Settings />} />}
        <Route path="/login" element={<Navigate to={home} replace />} />
        <Route path="*" element={<Navigate to={home} replace />} />
      </Routes>
    </Layout>
  );
}
