<p align="center">
  <img src="web/public/logo.svg" alt="HONE" width="120">
</p>

<h1 align="center">hone-quant</h1>

<p align="center">
  <strong>建立在 <a href="https://github.com/B-M-Capital-Research/honeclaw">honeclaw</a> 之上的自动化量化交易。</strong><br>
  研究员维护本体，AI 生成并执行策略，每一个决策都有据可查。
</p>

<p align="center">
  <strong>简体中文</strong> · <a href="README.md">English</a> ·
  <a href="https://hone-claw.com">官网</a> ·
  <a href="https://github.com/B-M-Capital-Research/honeclaw">honeclaw</a> ·
  <a href="#联系我们">联系我们</a>
</p>

---

hone-quant 把 honeclaw 的产业研究变成一套纪律严明、高度自动化的美股组合管理流程。honeclaw 的本体定义了 AI 基础设施投资范围（10 个板块、64 家公司）；hone-quant 在此之上决定仓位，每个交易日生成两份交易计划，在模拟账户中执行，并完整记录策略、计划、订单、成交和每一次人工决策，全程可审计。界面支持简体中文与英文切换，并同时显示新加坡时间和纽约时间。

> **仅限模拟交易。** hone-quant 不包含任何券商接口，无法下达真实订单。本项目内容不构成投资建议。

**目录：** [回测表现](#回测表现) · [研究员、本体与 Agent](#研究员本体与-agent) · [主要功能](#主要功能) ·
[界面截图](#界面截图) · [快速开始](#快速开始) · [一天的工作流程](#一天的工作流程) ·
[部署](#部署到-google-compute-engine) · [配置](#配置) · [命令行](#命令行) · [项目状态](#项目状态) ·
[代码结构](#代码结构) · [开发](#开发) · [文档](#文档) · [联系我们](#联系我们)

## 回测表现

![hone-claw.com/quant 的回测记录：板块优先 · 风险预算在近 3 年与近 1 年的总收益、年化、夏普、最大回撤和相对 QQQ 的超额收益](docs/screenshots/backtests-zh.png)

默认策略**板块优先 · 风险预算**在 hone-claw.com/quant 生产实例上、使用真实行情的回测结果：

| | **近 3 年** | **近 1 年** |
| --- | ---: | ---: |
| 区间 | 2023-10-05 → 2026-10-05 | 2025-10-05 → 2026-10-05 |
| 总收益 | **+535.88%** | **+70.69%** |
| 年化收益 | +85.24% | +71.01% |
| 夏普比率 | 2.10 | 1.59 |
| 最大回撤 | −28.01% | −24.57% |
| 相对 QQQ 超额收益 | **+421.46%** | +45.68% |

两次回测均使用 [Financial Modeling Prep](https://financialmodelingprep.com) 经拆股与分红调整的日线，每个交易日运行开盘计划和收盘前计划，并计入滑点、佣金和 SEC 规费；所用的策略引擎、调仓器和成本模型与模拟盘实际运行的完全相同。策略是内置的默认预设（`sector_risk_budget`，详见 [docs/methodology.zh.md](docs/methodology.zh.md)），任何拥有 FMP 密钥的人都可以在 **回测 → 新建回测** 中复现。

**请结合以下局限阅读这些数字**（应用在每份回测旁都会列出同样的说明）：

- **幸存者偏差。** 投资范围是本体中今天的成员名单。期间失败或未入选的公司不在样本中，而 2023–2026 年又是 AI 基础设施的非常时期，结果很可能偏乐观。
- **无未来数据。** 每次调仓的目标权重只用上一交易日收盘及之前的历史，加上下单时点的价格；历史不足的公司不会被配置。
- **回测不等于实盘记录。** 模拟盘账户于 2026-10-05 开始运行。过往表现与模拟表现都不代表未来收益。

## 研究员、本体与 Agent

```mermaid
flowchart LR
  R["研究员<br/>维护本体"] --> O["honeclaw 本体<br/>10 个板块 · 64 家公司"]
  O --> A["AI Agent<br/>生成并检验策略"]
  A --> E["hone-quant 引擎<br/>每个交易日两份计划"]
  E --> P["模拟账户<br/>订单 · 成交 · 审计日志"]
  P -- "绩效、归因、风险提醒" --> R
```

上面的结果来自一种分工：每一方只做自己最擅长的事。

- **专业研究员维护和运营本体。** 他们在 honeclaw 中为 AI 基础设施产业链建模：AI 芯片、存储、光互联、电力、数据中心等。每家公司都有明确的角色，模型还刻画了需求如何传导到它。本体决定 hone-quant *可以持有什么*。hone-quant 从不选股；本体成员的变化以差异清单的形式送达，经操作员审阅后才生效。
- **AI Agent 以本体为基础，严谨地生成和执行策略。** 策略沿着本体的结构展开：先分配板块预算（基于风险，兼顾动量和趋势），再在板块内按波动率倒数分配，单一公司上限 5%，市场宽度转弱时保留现金。策略用与实盘相同的引擎回测；每个策略都保存为不可修改的版本并记录启用历史，每份计划都能说明每个目标权重的由来。本仓库同样由 AI 编程 Agent 依据书面需求编写，配有 300 多项自动化测试与检查。
- **引擎提供不会走样的纪律。** 每个美股交易日两份计划，各有复核窗口；执行时检查报价新鲜度和价格偏离；换手上限和容忍带避免无谓交易；不加杠杆。人可以随时确认、取消或暂停，每一个操作都记入审计日志。

本体给了策略单凭价格得不到的结构：哪些公司属于同一环节，产业链的每一部分该承担多少风险。Agent 每天以同样的方式执行这套结构，不会疲倦，也不会事后动摇。引擎则确保决定的事情被准确地执行。

## 主要功能

- **一图看全部资产。** 总览页的主图把每家公司在所选区间（1 天至 1 年）的走势画成一根 K 线，按板块分组，下方同时显示当前权重与目标权重；点击任一 K 线即可查看该公司的 K 线图、均线、成交量和本账户的成交点。
- **相互独立的组合。** 可以同时运行多个模拟组合，用于不同的初始资金，或由不同的人各自管理自己的持仓。每个组合都有自己的持仓、现金、计划、成交、绩效、自动化模式，以及从共享策略库中选定的当前策略版本；投资范围与本体为所有组合共享。成员只能创建和管理自己的组合，管理员可以看到全部组合，只读用户只能查看。
- **每天两份计划，自动执行。** 开盘计划（开盘后 30 分钟）和收盘前计划（收盘前 3 小时），各有复核窗口。可全自动执行、人工确认或暂停；随时可以取消计划、移除单笔订单，或取消当日剩余交易。
- **透明的策略。** 每份计划都能说明每个目标权重的由来。策略以不可修改的版本保存并记录启用历史，内置四套预设。
- **贴近真实的模拟成交。** 报价新鲜度和价格偏离检查、滑点、佣金和 SEC 规费、整股交易、不加杠杆，并处理分红和拆股。
- **研究工具。** 回测使用同一套引擎和最长 10 年的复权日线，对比 SPY、QQQ、SMH 和范围等权基准，提供完整的风险收益指标并明确提示偏差；模拟盘的绩效分析包含按板块和公司的收益归因。
- **通知与提醒**（中文或英文）：站内消息、Telegram、飞书、企业微信、Slack、Discord、签名 Webhook 和邮件，支持免打扰时段、每日总结、每周报告和风险提醒。
- **便于运维。** 单个可执行文件内嵌网页界面；PostgreSQL 使用独立 schema（可与 honeclaw 共用实例）；提供面向 Google Compute Engine 的加固 systemd 部署、备份、审计日志和 CI。

## 界面截图

截图来自 hone-claw.com/quant 生产实例，使用真实行情。界面可随时切换为英文（右上角地球图标）。

![总览：每家公司一根 K 线、按板块分组，下方是当前与目标权重、当日计划和板块配置](docs/screenshots/overview-zh.png)

| | |
| --- | --- |
| ![成交与订单](docs/screenshots/trades-zh.png)<br>**成交与订单**：每份计划生成的每笔订单、成交情况与权重变化，可导出 CSV。 | ![交易计划](docs/screenshots/plan-zh.png)<br>**交易计划**：生成、复核窗口、执行的完整进度，以及每笔订单调整前 → 目标 → 调整后的权重。 |
| ![回测报告](docs/screenshots/backtest-zh.png)<br>**回测报告**：与模拟盘同一套引擎，收益、风险、基准对比和偏差提示。 | ![投资范围](docs/screenshots/universe-zh.png)<br>**投资范围**：来自 honeclaw 本体的板块与成员、交易限制和本体更新。 |
| ![设置](docs/screenshots/settings-zh.png)<br>**设置**：此处为计划时间，附双时区时间表预览。 | ![通知与提醒](docs/screenshots/notifications-zh.png)<br>**通知与提醒**：站内消息及 7 种外部渠道。 |

## 快速开始

在自己的电脑上运行有两种方式。两者都以**演示模式**启动：确定性的合成行情，不需要 API 密钥，界面上有明确标注；模拟账户初始资金 100 万美元。两种方式均于 2026-10-05 从零（全新克隆、空数据库）完整验证过。

### 方式 A：Docker Compose

需要：Docker Engine 及 Compose v2。

```bash
git clone https://github.com/B-M-Capital-Research/hone-quant.git
cd hone-quant
docker compose up --build        # 首次构建需要编译服务端，请预留几分钟
```

打开 <http://127.0.0.1:8090>，用 **admin** / **change-this-password** 登录。

- 自定首个密码：`HONE_QUANT_ADMIN_PASSWORD='一个足够长的口令' docker compose up --build`（至少 10 个字符；仅在还没有任何用户时生效）。
- Ctrl-C 停止并保留数据；`docker compose down -v` 删除容器和数据。
- PostgreSQL 映射在 `127.0.0.1:5434`，不会与本机已有实例冲突。

### 方式 B：从源码运行

需要：Rust 1.88+、Bun 1.3+、PostgreSQL 14+。

```bash
git clone https://github.com/B-M-Capital-Research/hone-quant.git
cd hone-quant

# 1. 创建角色和数据库（只需一次）。脚本通过标准输入传入，
#    因为 postgres 用户通常无权读取你主目录下的文件。
sudo -u postgres psql -v pw="'hone_quant_dev'" < deploy/postgres/setup.sql

# 2. 最小的 .env（从启动 hone-quant 的目录读取）。
cat > .env <<'EOF'
HONE_QUANT_DATABASE_URL=postgres://hone_quant:hone_quant_dev@127.0.0.1:5432/hone_quant
HONE_QUANT_MARKET_DATA=demo
HONE_QUANT_ADMIN_PASSWORD=change-this-password
EOF

# 3. 先构建网页界面（编译时嵌入可执行文件），再编译并运行服务端。
(cd web && bun install && bun run build)
cargo run --release -p quant-server -- serve
```

打开 <http://127.0.0.1:8090>，用 **admin** / **change-this-password** 登录。首次启动时 hone-quant 会执行数据库迁移、创建管理员和模拟账户、启用默认策略，并载入 10 年日线历史——演示模式下只需几秒。

### 使用真实行情（FMP）

hone-quant 的行情来自 [Financial Modeling Prep](https://financialmodelingprep.com)，与 honeclaw 使用同一家数据商。在 `.env` 中修改两行：

```bash
HONE_QUANT_MARKET_DATA=fmp
HONE_QUANT_FMP_API_KEY=你的密钥    # 或 HONE_QUANT_HONECLAW_CONFIG=/path/to/honeclaw/config.yaml
```

然后检查密钥并重新启动：

```bash
cargo run --release -p quant-server -- fmp-check   # 每个接口都应显示 OK
cargo run --release -p quant-server -- serve       # 首次启动会下载 10 年历史数据
```

使用 Docker 时：`HONE_QUANT_MARKET_DATA=fmp HONE_QUANT_FMP_API_KEY=你的密钥 docker compose up --build`。

演示数据和真实数据分别存放在不同的 schema（`hone_quant_demo` 和 `hone_quant`），切换时不会混在一起。真实账户从零开始，包括用户：首次启动时请在 `.env` 中保留 `HONE_QUANT_ADMIN_PASSWORD`，登录后再删除。想复现上面的回测，打开 **回测 → 新建回测**，选择「板块优先 · 风险预算」，区间选近 3 年或近 1 年。

### 在美股交易时段之外演练一个交易日

想在任何时间观看计划的生成和执行，可以用模拟时钟启动演示，并使用单独的 schema，避免模拟的一天与真实时钟的历史混在一起：

```bash
HONE_QUANT_DB_SCHEMA=hone_quant_rehearsal HONE_QUANT_DEV_CLOCK=2026-10-05T13:55:00Z \
  cargo run --release -p quant-server -- serve
```

时钟从纽约时间 09:55（新加坡 21:55）开始，按真实速度运行：10:00 生成开盘计划，10 分钟复核窗口后执行。仅限演示模式。

### 上手的前十分钟

1. **组合**——每个页面顶部的切换器决定当前操作的组合；在「组合 → 新建组合」中可以另开一个组合，设定它的初始资金、策略版本和自动化模式。
2. **总览**——K 线总览图按板块显示每家公司在所选区间的涨跌，下方是当前权重（灰色）和目标权重（红色刻度）。点击 K 线查看该公司的走势；「单一资产」可逐个浏览。
3. **策略**——「运作方式」用当前版本自己的参数逐步说明；「调参与试算」在保存新版本之前，先展示参数改动对今天目标权重的影响。
4. **回测 → 新建回测**——用当前策略或预设回测 1 至 10 年，对比 SPY、QQQ、SMH 和范围等权基准。
5. **设置**——计划时间、自动执行或人工确认、执行成本、风险提醒，以及「通知与渠道」：添加 Telegram、飞书、企业微信、Slack、Discord、Webhook 或邮件渠道并发送测试消息。
6. **交易计划**——计划生成时会通知你。打开即可看到每笔订单和每个目标权重的依据；复核窗口内可以取消计划或移除单笔订单。

## 一天的工作流程

以下为美国夏令时期间的默认新加坡时间（冬令时顺延一小时）。界面始终同时显示两个时区，所有时间都可以调整。

| 新加坡时间 | 系统动作 | 你需要做什么 |
| --- | --- | --- |
| 20:00 | 更新日线历史和公司行动（分红、拆股） | —— |
| 21:00 | 盘前简报：当天的计划时间和自动化模式 | 如今天不想交易，可以取消某个时段 |
| 22:00 | 生成开盘计划并通知 | 打开计划：订单、换手、成本，以及每个目标权重的依据 |
| 22:10 | 复核窗口结束后自动执行（自动模式） | 人工确认模式下由你确认；发现问题可以取消 |
| 01:00 | 收盘前计划（非紧急消息遵守免打扰时段） | 通常无需操作——自动模式加风险提醒即可覆盖夜间 |
| 04:15 | 收盘、记录日终净值，04:20 发送每日总结（周六另有每周报告） | 早上阅读总结 |

服务停机期间错过的时段会记为「错过」，绝不会延后补做。提前收盘日只运行开盘计划。

## 部署到 Google Compute Engine

完整手册见 **[docs/deployment-gce.zh.md](docs/deployment-gce.zh.md)**（[English](docs/deployment-gce.md)）。概要：

1. **数据库。** 在 honeclaw 已在使用的 PostgreSQL 实例上创建角色和独立数据库：`sudo -u postgres psql -v pw="'<强密码>'" < deploy/postgres/setup.sql`。hone-quant 的所有对象都在自己的 schema 中，从不触碰 honeclaw 的表。
2. **发布包。** 推送 `vX.Y.Z` 标签触发 GitHub Actions 的 *Release* 工作流，或在本地运行 `scripts/build-release.sh` → `dist/hone-quant-<version>-<revision>-linux-x86_64.tar.gz`。
3. **主机。** 把发布包复制到 VM，运行 `sudo scripts/install-host.sh`（系统用户、目录、加固的 systemd 单元、备份定时器），并填写 `/etc/hone-quant/runtime.env`：数据库地址、FMP 密钥、首个管理员。
4. **部署。** `sudo scripts/deploy.sh <发布包>` 会执行迁移、原子切换、健康检查，失败时自动回滚。然后运行 `sudo /opt/hone-quant/current/scripts/hq.sh fmp-check`。
5. **访问。** 不需要开放任何公网端口：`gcloud compute ssh <vm> --zone <zone> --tunnel-through-iap -- -N -L 8090:127.0.0.1:8090`，然后打开 <http://127.0.0.1:8090>。也可以选择用自己的域名提供 HTTPS（`deploy/caddy/`）。

每日定时任务会导出 `hone_quant` schema（保留最近 30 份）。请同时备份 `/var/lib/hone-quant/secret.key`：它用于加密已保存的通知渠道凭据。

### 部署在 hone-claw.com/quant

生产实例运行在 honeclaw 的虚拟机上，地址为 **https://hone-claw.com/quant**，没有自己的账号体系：hone-quant 在服务端校验访问者的 hone-claw.com 登录状态，只允许管理员进入。Cloudflare Worker 把 `/quant*` 连同共享的源站令牌转发到源站。部署手册与回滚步骤见 **[docs/deploy-hone-claw-quant.md](docs/deploy-hone-claw-quant.md)**（英文）。

## 配置

部署相关的事实用环境变量配置——本地运行用 `.env`，生产环境用 `/etc/hone-quant/runtime.env`。运行中需要调整的一切（计划时间、自动化模式、成本、风险提醒、通知渠道、提醒事项、策略版本）都在网页界面中修改，保存在 PostgreSQL 里并记入审计日志。

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `HONE_QUANT_DATABASE_URL` | —— | PostgreSQL 连接；也可用 `HONE_QUANT_PG_*`，或 honeclaw 的 `DATABASE_URL` / `HONE_POSTGRES_*` |
| `HONE_QUANT_DB_SCHEMA` | `hone_quant`（演示模式为 `hone_quant_demo`） | 存放 hone-quant 所有表的 schema |
| `HONE_QUANT_MARKET_DATA` | `fmp` | `fmp`（真实行情）或 `demo`（合成行情） |
| `HONE_QUANT_FMP_API_KEY` | —— | FMP 密钥；`HONE_QUANT_FMP_API_KEYS` 可轮换多个，`HONE_QUANT_HONECLAW_CONFIG` 可复用 honeclaw 的配置 |
| `HONE_QUANT_BIND` | `127.0.0.1:8090` | 监听地址（建议只监听本机，通过隧道或反向代理访问） |
| `HONE_QUANT_STATE_DIR` | `./data` | 存放 `secret.key`，用于加密通知渠道凭据 |
| `HONE_QUANT_ADMIN_USER` / `HONE_QUANT_ADMIN_PASSWORD` | `admin` / —— | 首个管理员，仅在还没有任何用户时创建 |
| `HONE_QUANT_INITIAL_CASH` | `1000000` | 模拟账户初始资金 |
| `HONE_QUANT_PUBLIC_URL` | —— | 反向代理后的公网 HTTPS 地址：用于通知中的链接，也是写操作唯一接受的 `Origin` |
| `HONE_QUANT_BASE_PATH` | —— | 部署在某个路径下，例如 `/quant`（网页需用同一值构建） |
| `HONE_QUANT_AUTH_MODE` | `local` | `honeclaw`：不使用本地账号，只允许 honeclaw 管理员，在服务端校验（`HONE_QUANT_HONECLAW_*`） |
| `HONE_QUANT_ORIGIN_TOKEN` | —— | 前置代理必须以 `X-Hone-Quant-Origin-Token` 发送的密钥，否则一律返回 404 |
| `HONE_QUANT_DEV_CLOCK` | —— | 仅演示模式：让时钟从指定时刻开始（RFC 3339） |

全部变量的说明见 [`deploy/runtime.env.example`](deploy/runtime.env.example)。

## 命令行

```text
hone-quant serve                        网页界面、API、调度器和模拟券商（默认命令）
hone-quant migrate                      执行数据库迁移后退出
hone-quant user add <name> [--role admin|member|viewer]
hone-quant user passwd <name>           设置新密码并让该用户的会话全部退出
hone-quant user list
hone-quant fmp-check [--symbol NVDA]    逐一检查 hone-quant 用到的 FMP 接口
hone-quant sync [--full]                立即拉取行情
hone-quant universe sync [--apply]      对比 honeclaw 本体与数据库中的投资范围
hone-quant universe build --edits <honeclaw>/data/industry_map/edits.json
```

从源码运行时在前面加 `cargo run --release -p quant-server --`。在部署好的主机上，用服务的用户和环境运行：`sudo /opt/hone-quant/current/scripts/hq.sh <命令>`。

## 项目状态

*截至 2026-10-06。*

- **已上线生产**：自 2026-10-05 起运行在 https://hone-claw.com/quant，仅限 hone-claw.com 管理员使用。数据库使用 honeclaw 的 PostgreSQL（独立角色和数据库），行情为真实 FMP 数据（全部 67 个代码的十年日线）。
- **需求说明中的功能已全部实现**：本体投资范围、仅凭价格决定权重、每个交易日两份计划、在模拟账户中自动执行、人工干预（取消计划、移除订单、取消当日剩余交易、人工确认模式、暂停、切换策略版本）、通知与提醒、可配置的设置、完整的审计记录、回测、绩效分析、中英文界面，以及 GCE 部署工具。
- **测试全部通过**：82 个引擎测试；201 个服务端测试，包括 PostgreSQL 集成测试、针对模拟 FMP 服务器的行情客户端测试、通知渠道测试和针对模拟 honeclaw 的登录校验测试；22 个网页单元测试和 5 个浏览器端到端测试；Cloudflare Worker 的 15 个测试，以及其部署脚本针对模拟 Cloudflare API 的 22 项检查。`cargo clippy` 无警告。
- **仍在起步阶段**：模拟盘账户于 2026-10-05 开始运行，实盘年化指标需要 21 个交易日才显示、约 63 个交易日才可靠，在此之前请参考回测。外部通知渠道目前只针对本地模拟服务测试过。

**有意不做的事**

- **实盘交易**——没有券商接口，也没有任何设置可以开启。
- **选股**——投资范围来自 honeclaw 本体，hone-quant 只决定仓位大小。
- **日内信号**——策略使用日收盘价加当前报价；回测使用日线。

## 代码结构

```text
crates/quant-core     策略、调仓、成本、时间表、绩效指标和回测：纯 Rust，无 I/O
crates/quant-server   axum API、调度器、模拟券商、FMP 客户端、通知、PostgreSQL 迁移
web/                  SolidJS + ECharts 网页界面（中/英），嵌入服务端可执行文件
config/               由 honeclaw 本体生成的内置投资范围快照
deploy/               systemd 单元、runtime.env 模板、PostgreSQL 初始化脚本、Caddy 示例与
                      hone-claw.com/quant 路由脚本、Cloudflare Worker（cloudflare/quant-proxy）
scripts/              发布包构建、主机安装、带回滚的部署、备份、hq.sh 包装脚本
docs/                 方法说明、系统架构、GCE 部署手册（中/英）、hone-claw.com/quant 部署手册、截图
```

## 开发

```bash
# Rust（集成测试需要一个可以创建 schema 的 PostgreSQL 数据库）
export HONE_QUANT_TEST_DATABASE_URL=postgres://hone_quant:…@127.0.0.1:5432/hone_quant
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace

# 网页：类型检查、单元测试、生产构建
(cd web && bun run typecheck && bun run test && bun run build)

# 界面开发（热更新）：一个终端运行 API，另一个终端在 http://127.0.0.1:5173 运行界面
cargo run -p quant-server
(cd web && bun run dev)

# 针对运行中的应用做浏览器端到端测试和截图（建议使用演示数据）
(cd web && HONE_QUANT_E2E_URL=http://127.0.0.1:8090 HONE_QUANT_E2E_PASSWORD=… bun run test:e2e)
(cd web && HONE_QUANT_SHOT_PASSWORD=… node scripts/screenshot.mjs --base http://127.0.0.1:8090 --pages /,/plans --locales zh,en --themes light,dark)
```

内置投资范围可以从 honeclaw 本体重新生成：`hone-quant universe build --edits <honeclaw>/data/industry_map/edits.json`。

## 文档

- [策略与交易方法](docs/methodology.zh.md)（[English](docs/methodology.md)）
- [系统架构](docs/architecture.md)
- [GCE 部署手册](docs/deployment-gce.zh.md)（[English](docs/deployment-gce.md)）
- [hone-claw.com/quant 生产部署](docs/deploy-hone-claw-quant.md)（英文）及其 [Cloudflare Worker](deploy/cloudflare/quant-proxy/README.md)
- 配置项说明：[`deploy/runtime.env.example`](deploy/runtime.env.example)

## 联系我们

我们是一家总部位于新加坡的投资研究机构，也是 [honeclaw](https://github.com/B-M-Capital-Research/honeclaw) 和 hone-quant 背后的团队。如果你对这些开源项目感兴趣，无论是想使用、在此基础上开发，还是与我们合作，都欢迎来信。我们同时提供社区咨询、投研指导及相关服务。

- **邮箱：** [contact@honeclaw.app](mailto:contact@honeclaw.app)
- **官网：** [hone-claw.com](https://hone-claw.com)

也欢迎在本仓库提交 Issue 和 Pull Request。

## 许可证

[MIT](LICENSE)，与 honeclaw 相同。本仓库的任何内容都不构成投资建议。
