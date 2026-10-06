#!/opt/pwlogin/bin/python
"""辅助登录 worker:无头浏览器替用户走 Azure 授权码登录(账号密码 + MFA 数字匹配),
用 App 自带的 https://localhost 回调即时截获授权码并换取 Azure token,绕开被租户封禁的设备码流程。

输入:stdin 一行 JSON {"session_id","username","password"}
输出:状态文件 /tmp/assisted/<session_id>.json,字段 stage=starting|mfa|success|failed,
      成功时含 az_access_hex / az_refresh_hex(AES-GCM,服务器 ENCRYPTION_KEY 加密,nonce12||ct 的 hex)。
密码只在本进程内存使用,不落盘、不打印。
"""
import json, os, re, sys, time, urllib.parse, urllib.request, urllib.error
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from playwright.sync_api import sync_playwright

TENANT = "274313da-18e1-40ab-97e0-adc6eb1ec699"
CLIENT = "e9ed2cb6-da5d-48dc-b8be-28b0d6016e53"
RESOURCE = "https://graph.microsoft.com/"
REDIRECT = "https://localhost"
STATUS_DIR = "/tmp/assisted"
ENV_PATH = "/opt/instatt_saas/.env"
DOMAIN = "nottingham.edu.my"
MFA_WAIT_SECS = 180

def enc_key():
    for line in open(ENV_PATH):
        if line.startswith("ENCRYPTION_KEY="):
            return line.split("=", 1)[1].rstrip("\r\n").encode()
    raise RuntimeError("no ENCRYPTION_KEY")

def encrypt_hex(key, s):
    nonce = os.urandom(12)
    ct = AESGCM(key).encrypt(nonce, s.encode(), None)
    return (nonce + ct).hex()

def main():
    req = json.loads(sys.stdin.readline())
    sid = req["session_id"]
    raw = (req.get("username") or "").strip()
    username = raw if "@" in raw else f"{raw}@{DOMAIN}"
    password = req.get("password") or ""
    if not re.fullmatch(r"[A-Za-z0-9]{8,64}", sid):
        return  # 非法 session_id,直接退出
    os.makedirs(STATUS_DIR, exist_ok=True)
    path = os.path.join(STATUS_DIR, f"{sid}.json")
    st = {"stage": "starting"}
    def save():
        tmp = path + ".tmp"
        json.dump(st, open(tmp, "w"))
        os.replace(tmp, path)
    save()

    KEY = enc_key()
    AUTH = (f"https://login.microsoftonline.com/{TENANT}/oauth2/authorize?" +
            urllib.parse.urlencode({"client_id": CLIENT, "response_type": "code",
                                    "redirect_uri": REDIRECT, "resource": RESOURCE, "prompt": "login"}))
    captured = {"code": None}

    def exchange(code):
        r = urllib.request.urlopen(urllib.request.Request(
            f"https://login.microsoftonline.com/{TENANT}/oauth2/token",
            data=urllib.parse.urlencode({"grant_type": "authorization_code", "client_id": CLIENT,
                "code": code, "redirect_uri": REDIRECT, "resource": RESOURCE}).encode()), timeout=30)
        return json.load(r)

    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(headless=True, args=["--no-sandbox"])
            ctx = browser.new_context(locale="en-US", ignore_https_errors=True)
            def route_cb(route):
                u = route.request.url
                if u.startswith("https://localhost") and "code=" in u and not captured["code"]:
                    q = urllib.parse.urlparse(u).query
                    captured["code"] = urllib.parse.parse_qs(q).get("code", [None])[0]
                try: route.fulfill(status=200, body="ok")
                except Exception:
                    try: route.abort()
                    except Exception: pass
            ctx.route(re.compile(r"^https://localhost"), route_cb)
            page = ctx.new_page()
            def on_nav(frame_url):
                if frame_url.startswith("https://localhost") and "code=" in frame_url and not captured["code"]:
                    captured["code"] = urllib.parse.parse_qs(urllib.parse.urlparse(frame_url).query).get("code", [None])[0]
            page.on("framenavigated", lambda f: on_nav(f.url))
            page.on("request", lambda r: on_nav(r.url) if r.url.startswith("https://localhost") else None)

            def fill(sels, val):
                for s in sels:
                    try: page.wait_for_selector(s, timeout=6000, state="visible").fill(val); return True
                    except Exception: continue
                return False
            def click(sels):
                for s in sels:
                    try: page.wait_for_selector(s, timeout=6000, state="visible").click(); return True
                    except Exception: continue
                return False

            page.goto(AUTH, wait_until="domcontentloaded", timeout=60000)
            time.sleep(1.2)
            fill(["input[type=email]", "input[name=loginfmt]", "input#i0116"], username)
            click(["input#idSIButton9", "input[type=submit]"])

            # 等待:密码页 / MFA 数字匹配页 / (直接回调)
            for _ in range(14):
                time.sleep(1.5)
                if captured["code"]:
                    break
                body = ""
                try: body = page.inner_text("body")
                except Exception: pass
                if "Enter the number" in body or "Approve sign" in body:
                    m = re.search(r"Enter the number[^\d]*(\d{1,3})", body) or re.search(r"\n\s*(\d{2,3})\s*\n", body)
                    st["stage"] = "mfa"; st["mfa_number"] = m.group(1) if m else None; save()
                    break
                if "password" in body.lower() and page.query_selector("input[type=password]"):
                    if not password:
                        st["stage"] = "failed"; st["error"] = "该账号需要密码,请在登录表单填写密码"; save()
                        browser.close(); return
                    fill(["input[type=password]", "input[name=passwd]", "input#i0118"], password)
                    click(["input#idSIButton9", "input[type=submit]"])
                if "account or password is incorrect" in body.lower() or "AADSTS50126" in body:
                    st["stage"] = "failed"; st["error"] = "账号或密码错误"; save(); browser.close(); return

            # 等用户在 Authenticator 批准 → 截获回调;期间点过确认/保持登录页
            deadline = time.time() + MFA_WAIT_SECS
            while time.time() < deadline and not captured["code"]:
                for s in ["input#idSIButton9", "button:has-text('Continue')", "button:has-text('Yes')"]:
                    try:
                        if page.is_visible(s): page.click(s, timeout=1000)
                    except Exception: pass
                try:
                    b = page.inner_text("body")
                    if ("didn't approve" in b.lower() or "we didn't hear" in b.lower()
                            or "request was denied" in b.lower() or "AADSTS50074" in b or "AADSTS500121" in b):
                        st["stage"] = "failed"; st["error"] = "验证未通过或超时,请重试"; save(); browser.close(); return
                except Exception: pass
                time.sleep(2)

            if not captured["code"]:
                st["stage"] = "failed"; st["error"] = "等待验证超时,请重试"; save(); browser.close(); return

            tok = exchange(captured["code"])
            browser.close()
            acc = tok.get("access_token"); ref = tok.get("refresh_token")
            if not acc or not ref:
                st["stage"] = "failed"; st["error"] = "换取令牌失败"; save(); return
            st = {"stage": "success",
                  "az_access_hex": encrypt_hex(KEY, acc),
                  "az_refresh_hex": encrypt_hex(KEY, ref)}
            # overwrite (drop mfa_number etc.)
            json.dump(st, open(path + ".tmp", "w")); os.replace(path + ".tmp", path)
    except Exception as ex:
        st = {"stage": "failed", "error": f"登录异常: {str(ex)[:120]}"}
        try: json.dump(st, open(path + ".tmp", "w")); os.replace(path + ".tmp", path)
        except Exception: pass

if __name__ == "__main__":
    main()
