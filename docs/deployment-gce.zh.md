# 在 Google Compute Engine 上部署 hone-quant

本手册说明如何把 hone-quant 部署到 Google Compute Engine（GCE）的 Linux 虚拟机上，与 honeclaw 同机运行并共用其 PostgreSQL 实例。English: [deployment-gce.md](deployment-gce.md)。部署在 hone-claw.com/quant 的生产实例（仅限 honeclaw 管理员，前置 Cloudflare Worker）另有手册：[deploy-hone-claw-quant.md](deploy-hone-claw-quant.md)（英文）。

hone-quant 是一个可执行文件（网页界面已内嵌），由 systemd 管理，只监听 `127.0.0.1:8090`；通过 IAP/SSH 隧道（推荐）或 HTTPS 反向代理访问。**它只会在模拟盘中交易**——代码中没有任何券商接口，也没有任何设置可以开启实盘。

```
 你的电脑 ──IAP 隧道──▶ 虚拟机：hone-quant（127.0.0.1:8090，systemd）
                              ├─▶ PostgreSQL（与 honeclaw 共用实例；schema 为 hone_quant）
                              ├─▶ financialmodelingprep.com（行情，HTTPS）
                              └─▶ Telegram / 飞书 / 企业微信 / Slack / Discord / 邮件（通知）
```

## 1. 环境要求

| 项目 | 建议 |
| --- | --- |
| 虚拟机 | e2-small（2 vCPU、2 GB）或更高；Debian 12 或 Ubuntu 22.04/24.04（x86_64） |
| 磁盘 | 预留 10 GB 给数据库（约 70 个代码、10 年日线远小于 1 GB）、发布包和备份 |
| PostgreSQL | 14 或更新版本；直接使用 honeclaw 已有的实例即可 |
| 网络 | 需要出站 HTTPS。使用 IAP 隧道时无需开放任何入站端口 |
| 行情 | Financial Modeling Prep 密钥，需包含美股报价、日线历史和分钟线 |
| 时钟 | GCE 镜像默认通过元数据服务器校时，请保持开启（计划时间依赖系统时钟） |

主机时区无关紧要：市场逻辑始终使用 `America/New_York`，界面同时显示新加坡时间（可配置）和纽约时间。

## 2. PostgreSQL

hone-quant 的所有对象都放在独立的 schema（`hone_quant`）中，从不读写 honeclaw 的表。用 PostgreSQL 超级用户执行一次：

```bash
sudo -u postgres psql -v pw="'<强密码>'" < deploy/postgres/setup.sql
```

脚本通过标准输入传入，因为 `postgres` 用户通常无权读取你主目录下的文件（用 `-f` 会报 "Permission denied"）。`setup.sql` 默认在共享实例上创建独立的 `hone_quant` 数据库（推荐），也写明了另一种做法：在 honeclaw 的数据库里建一个 `hone_quant` schema。首次启动时，表结构会通过带校验和的版本化迁移自动创建。

如果虚拟机环境里已经有 honeclaw 的 `DATABASE_URL` 或 `HONE_POSTGRES_*`，hone-quant 也可以直接沿用（对象仍然放在 `hone_quant` schema 中）；但更推荐为它单独配置 `HONE_QUANT_DATABASE_URL` 和独立的数据库角色，便于审计。

## 3. 构建发布包

发布包由 GitHub Actions 的 **Release** 工作流在 Ubuntu 22.04 上构建（因此可在 Debian 12 和 Ubuntu 22.04+ 上运行）：推送 `vX.Y.Z` 标签会生成 GitHub Release，也可以手动运行工作流后下载构建产物。如需在本地构建（需要 Rust 和 Bun）：

```bash
scripts/build-release.sh
# → dist/hone-quant-<版本>-<提交>-linux-x86_64.tar.gz 及 .sha256
```

## 4. 首次安装

把发布包复制到虚拟机并初始化主机（只需一次）：

```bash
gcloud compute scp dist/hone-quant-*.tar.gz* <vm>:/tmp/ --zone <zone> --tunnel-through-iap
gcloud compute ssh <vm> --zone <zone> --tunnel-through-iap

# 在虚拟机上
cd /tmp && tar -xzf hone-quant-*.tar.gz && cd hone-quant-*/
sudo scripts/install-host.sh            # 创建用户、目录、systemd 单元和 runtime.env 模板
sudo apt-get install -y postgresql-client curl   # 备份需要 pg_dump，健康检查需要 curl
sudoedit /etc/hone-quant/runtime.env
```

`runtime.env` 中至少需要设置：

- `HONE_QUANT_DATABASE_URL`：第 2 步创建的角色和数据库；
- `HONE_QUANT_FMP_API_KEY`（或用 `HONE_QUANT_HONECLAW_CONFIG` 指向 honeclaw 的 `config.yaml`，复用其中 `fmp:` 的密钥）；
- `HONE_QUANT_ADMIN_USER` / `HONE_QUANT_ADMIN_PASSWORD`：第一个管理员（首次登录后请删除密码这一行）。

全部变量的说明见 `deploy/runtime.env.example`。然后部署并启动：

```bash
sudo scripts/deploy.sh /tmp/hone-quant-<版本>-<提交>-linux-x86_64.tar.gz
sudo /opt/hone-quant/current/scripts/hq.sh fmp-check     # 每个接口都应显示 OK
```

首次启动会执行数据库迁移、载入投资范围（来自 honeclaw 本体的 10 个板块、64 家公司）、创建模拟账户（默认 100 万美元）、启用默认策略，并下载 10 年日线历史（需要几分钟）。

## 5. 访问

**IAP 隧道（推荐，无需公开端口）。** 先放行 IAP 网段访问 SSH（只需一次）：

```bash
gcloud compute firewall-rules create allow-iap-ssh --network <network> \
  --direction INGRESS --action allow --rules tcp:22 --source-ranges 35.235.240.0/20
```

然后在自己的电脑上执行：

```bash
gcloud compute ssh <vm> --zone <zone> --tunnel-through-iap -- -N -L 8090:127.0.0.1:8090
```

打开 <http://127.0.0.1:8090> 即可。按 Ctrl-C 关闭隧道。

**使用自己的域名和 HTTPS（可选）。** 在前面加一层 Caddy（或 nginx），参考 `deploy/caddy/Caddyfile.example`，并设置 `HONE_QUANT_PUBLIC_URL=https://…` 和 `HONE_QUANT_SECURE_COOKIE=true`。建议再加一道访问控制（Cloudflare Access、HTTPS 版 Google IAP 或来源 IP 白名单）——私人交易台没有必要暴露在公网上。

## 6. 日常运维

| 操作 | 命令 |
| --- | --- |
| 状态 / 日志 | `systemctl status hone-quant` · `journalctl -u hone-quant -f` |
| 健康检查 | `curl -s 127.0.0.1:8090/api/health` → `{"ok":true,"db":true,"version":…,"revision":…}` |
| 添加操作员 | `sudo /opt/hone-quant/current/scripts/hq.sh user add alice --role viewer` |
| 重置密码 | `sudo …/hq.sh user passwd alice`（会让该用户的现有会话失效） |
| 立即同步行情 | `sudo …/hq.sh sync`（加 `--full` 重新下载全部历史） |
| 更新投资范围 | 投资范围页 →「检查更新」，或 `sudo …/hq.sh universe sync --apply` |
| 升级 | `sudo scripts/deploy.sh <新发布包>`（自动迁移、切换、健康检查，失败自动回滚） |
| 手动回滚 | `sudo ln -sfn /opt/hone-quant/releases/<上一版本> /opt/hone-quant/current && sudo systemctl restart hone-quant` |

其余日常设置——计划时间、自动交易模式、交易成本、风险提醒、通知渠道、提醒事项、策略版本——都在网页界面中管理，每一次修改都会写入审计日志。

**每日时间表（纽约时间）。** 08:00 盘前同步行情；10:00 生成开盘计划（开盘后 30 分钟）；13:00 生成收盘前计划（收盘前 3 小时）；每份计划生成后有复核窗口（默认 10 分钟），之后自动执行；16:15 收盘后同步并记录日终净值，随后发送每日总结。提前收盘日（13:00 收盘）只运行开盘计划。服务停机期间错过的时段会被记录为「错过」，不会事后补单。

## 7. 备份

`hone-quant-backup.timer` 每天 22:30 UTC（美股收盘后）运行 `scripts/backup.sh`，在 `/var/backups/hone-quant` 中保留最近 30 份 `hone_quant` schema 的备份。同时请备份 `/var/lib/hone-quant/secret.key`——缺少它时，已保存的通知渠道凭据无法解密（其他功能不受影响，只需重新填写渠道配置）。

```bash
sudo systemctl start hone-quant-backup.service      # 立即备份一次
sudo ls -l /var/backups/hone-quant
# 恢复（先停止服务）
sudo systemctl stop hone-quant
sudo -u hone-quant pg_restore --clean --if-exists --no-owner -d "<数据库连接串>" <备份文件>
sudo systemctl start hone-quant
```

GCE 永久磁盘快照可以作为第二层保护。

## 8. 常见问题

| 现象 | 排查 |
| --- | --- |
| 服务启动失败 | `journalctl -u hone-quant -n 100`：配置错误会指明需要修改的变量 |
| 提示 "cannot create schema" | 数据库角色需要 `CREATE` 权限，或按 setup.sql 的方案 B 预先建好 schema |
| 收到行情过期提醒 | 运行 `hq.sh fmp-check`；FMP 套餐限制（HTTP 402/403）和限流都会在这里显示 |
| 计划被记录为「错过」 | 计划时间点服务未运行或系统时钟不准；见 设置 → 数据 → 任务记录 |
| 计划「已过期」 | 人工确认模式下截止前无人确认——改为自动执行，或提前确认 |
| 收不到通知 | 设置 → 通知 → 对应渠道 →「发送测试」；免打扰时段会延后非紧急消息 |
