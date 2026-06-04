#!/usr/bin/env python3
"""在部署服务器上执行命令并实时回显输出,退出码透传。
用法: python scripts/rexec.py "shell 命令"
连接参数从 .deploy.env 读取(HOST/USER/PASS/REMOTE_DIR)。
"""
import sys, os, paramiko

def load_env():
    env = {}
    path = os.path.join(os.path.dirname(__file__), "..", ".deploy.env")
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                env[k.strip()] = v.strip()
    return env

def main():
    cmd = sys.argv[1]
    env = load_env()
    c = paramiko.SSHClient()
    c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(env["HOST"], username=env["USER"], password=env["PASS"], timeout=30)
    # 默认进入 REMOTE_DIR(若存在),便于跑 cargo 等
    remote_dir = env.get("REMOTE_DIR", "")
    full = f"cd {remote_dir} 2>/dev/null; {cmd}" if remote_dir else cmd
    chan = c.get_transport().open_session()
    chan.get_pty()
    chan.exec_command(full)
    while True:
        if chan.recv_ready():
            sys.stdout.write(chan.recv(4096).decode(errors="replace"))
            sys.stdout.flush()
        if chan.exit_status_ready() and not chan.recv_ready():
            break
    # 收尾
    while chan.recv_ready():
        sys.stdout.write(chan.recv(4096).decode(errors="replace"))
    code = chan.recv_exit_status()
    c.close()
    sys.exit(code)

if __name__ == "__main__":
    main()
