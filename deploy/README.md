# 部署说明(Ubuntu 24.04)

后端为单个 Rust 二进制,自带 SPA 静态托管 + HTTP API;PostgreSQL 本机;nginx 做 80 端口入口。

## 目录与构建

```bash
# 源码位于 /opt/instatt_saas(由 scripts/sync.py 同步)
cd /opt/instatt_saas
# 后端 release 构建
set -a; . ./.env; set +a
cargo build --release
# 前端构建
cd web && npm install && npm run build && cd ..
```

## 环境变量 /opt/instatt_saas/.env

```
DATABASE_URL=postgres://instatt:instatt@localhost:5432/instatt
JWT_SECRET=<openssl rand -hex 32>
ENCRYPTION_KEY=<恰好 32 字节,如 openssl rand -hex 16>
SUPERADMIN_USERNAME=admin
SUPERADMIN_PASSWORD=changeme-strong-pass        # 首次启动 seed,生产请尽快在面板改掉
BIND_ADDR=127.0.0.1:8080
WEB_DIR=/opt/instatt_saas/web/dist
COOKIE_SECURE=false                  # 配好 HTTPS 后改为 true
RUST_LOG=instatt_saas=info
```

> 启动时自动跑数据库迁移并 seed 超管 + 默认配置,无需手动建表。

## systemd

```bash
cp deploy/instatt.service /etc/systemd/system/instatt.service
systemctl daemon-reload
systemctl enable --now instatt
systemctl status instatt        # 查看状态
journalctl -u instatt -f        # 跟踪日志
```

## nginx

```bash
cp deploy/nginx-instatt.conf /etc/nginx/sites-available/instatt
ln -sf /etc/nginx/sites-available/instatt /etc/nginx/sites-enabled/instatt
rm -f /etc/nginx/sites-enabled/default
nginx -t && systemctl reload nginx
```

访问 `http://<服务器IP>/`。

## HTTPS(可选,需域名)

```bash
apt-get install -y certbot python3-certbot-nginx
certbot --nginx -d your.domain.com
# 然后把 .env 的 COOKIE_SECURE 改为 true 并 systemctl restart instatt
```

## 数据库备份(每日 pg_dump)

```bash
mkdir -p /opt/backups
cat >/etc/cron.d/instatt-backup <<'EOF'
0 3 * * * root PGPASSWORD=instatt pg_dump -h localhost -U instatt instatt | gzip > /opt/backups/instatt-$(date +\%F).sql.gz 2>>/var/log/instatt-backup.log; find /opt/backups -name 'instatt-*.sql.gz' -mtime +14 -delete
EOF
```

## 更新发布

```bash
# 本地:python scripts/sync.py
cd /opt/instatt_saas
set -a; . ./.env; set +a
cargo build --release
cd web && npm run build && cd ..
systemctl restart instatt
```
