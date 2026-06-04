#!/usr/bin/env python3
"""把本地源码同步到部署服务器的 REMOTE_DIR(打 tar 上传后解压)。
排除 target/.git/node_modules/web/dist 等重目录。
用法: python scripts/sync.py
"""
import os, io, tarfile, paramiko

EXCLUDE_DIRS = {".git", "target", "node_modules", "dist", "pgdata"}
# 不外传:真实用户 token、部署口令、本地 env。服务器自带 .env(见 scripts/rexec 部署)。
EXCLUDE_FILES = {".deploy.env", ".env", "instatt_tokens.json"}

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

def build_tar(root):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames[:] = [d for d in dirnames if d not in EXCLUDE_DIRS]
            for fn in filenames:
                if fn in EXCLUDE_FILES:
                    continue
                full = os.path.join(dirpath, fn)
                rel = os.path.relpath(full, root)
                tar.add(full, arcname=rel)
    buf.seek(0)
    return buf

def main():
    env = load_env()
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    remote_dir = env["REMOTE_DIR"]
    tar_buf = build_tar(root)

    c = paramiko.SSHClient()
    c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(env["HOST"], username=env["USER"], password=env["PASS"], timeout=30)
    c.exec_command(f"mkdir -p {remote_dir}")[1].channel.recv_exit_status()
    sftp = c.open_sftp()
    sftp.putfo(tar_buf, "/tmp/instatt_sync.tar.gz")
    sftp.close()
    stdin, stdout, stderr = c.exec_command(
        f"tar xzf /tmp/instatt_sync.tar.gz -C {remote_dir} && rm /tmp/instatt_sync.tar.gz && echo SYNC_OK"
    )
    code = stdout.channel.recv_exit_status()
    print(stdout.read().decode() + stderr.read().decode())
    c.close()
    raise SystemExit(code)

if __name__ == "__main__":
    main()
