# EXLOLI-IMGX

基于 Rust 和 Teloxide 的 E-Hentai / ExHentai Telegram 频道发布与画廊管理机器人。

它会按配置定期扫描 E-Hentai / ExHentai 搜索结果，下载尚未收录画廊的图片到 K-Vault 图床，生成 Telegraph 图片预览页，并发布到 Telegram 频道。机器人同时提供画廊查询、排行、随机检索、收藏、评分和管理员维护命令。

> 本项目需要有效的 ExHentai 登录 Cookie、Telegram Bot Token、Telegraph Access Token 和 K-Vault API Token。它们均属于敏感数据，必须只保存在本机 `config.toml` 中，不能提交到 Git。

## 功能

- 定时扫描：按自定义 E-Hentai 搜索条件获取最新画廊。
- 图片处理：下载画廊图片，上传到 K-Vault，并以图片哈希去重。
- 内容发布：创建 Telegraph 预览文章，向指定 Telegram 频道发送标题、标签、预览和来源链接。
- 数据管理：使用 SQLite 保存画廊、图片、频道消息、Telegraph 链接、评分、收藏和挑战记录。
- 社群互动：频道消息转发到讨论组后可生成评分面板；支持排行榜、个人收藏、随机画廊和猜画师。
- 管理维护：管理员可上传新画廊、软删除、硬删除和重建预览。

## 运行流程

```text
ExHentai 搜索 → 解析画廊 → 下载图片 → K-Vault 上传 → SQLite 建档
                                              ↓
                                   Telegraph 预览页 → Telegram 频道
```

`search_count = 0` 时不会自动扫描和上传，但 Telegram Bot 仍会运行，可用于查询、收藏、评分和管理员维护。

## 部署

生产环境使用 Docker Compose。镜像：`ghcr.io/aiza2a/exloli-imgx:latest`。

### 1. 准备目录和文件

```sh
mkdir -p exloli-img && cd exloli-img
curl -fsSLO https://raw.githubusercontent.com/aiza2a/exloli-img/master/docker-compose.yml
curl -fsSLo config.toml https://raw.githubusercontent.com/aiza2a/exloli-img/master/config.toml.example
curl -fsSLO https://github.com/EhTagTranslation/Database/releases/latest/download/db.text.json
```

首次运行前不需要手动创建 SQLite 文件；程序会按 `database_url` 自动创建并执行迁移。

### 2. 配置 `config.toml`

至少填写以下字段：

- `[exhentai].cookie`：可访问 ExHentai 的完整 Cookie。
- `[telegraph].access_token`：Telegraph 帐号 Access Token。
- `[telegram].channel_id`、`group_id`、`auth_group_id`、`bot_id`、`token`。
- `[kvault].base_url`、`api_token`：K-Vault 服务地址及上传 Token。

建议先设置：

```toml
[exhentai]
search_count = 0
```

确认 Bot 能正常响应 `/ping` 后，再逐步增加 `search_count`。自动扫描会产生 ExHentai、K-Vault、Telegraph 与 Telegram 的外部请求。

### 3. 启动和查看状态

```sh
docker compose pull
docker compose up -d
docker compose ps
docker compose logs -f --tail=100
```

升级镜像时不要使用 `docker compose down -v`；这会删除具名 volume。当前 Compose 将 SQLite、标签翻译文件和配置文件作为宿主机文件挂载，升级前仍建议先备份。

## 配置参考

```toml
log_level = "info,sqlx=warn,teloxide=error,exloli_next=debug"
threads_num = 3
interval = "1h"
database_url = "db.sqlite"

[exhentai]
cookie = "ipb_member_id=...; ipb_pass_hash=...; igneous=..."
search_params = [
  ["f_cats", "577"],
  ["f_search", "language:Chinese"]
]
search_count = 0
trans_file = "db.text.json"

[telegraph]
access_token = "..."
author_name = "exloli"
author_url = "https://t.me/your_channel"

[telegram]
channel_id = "@your_channel"
group_id = -1000000000000
auth_group_id = -1000000000000
bot_id = "your_bot_username"
token = "123456:telegram-bot-token"

[kvault]
base_url = "https://files.example.com"
api_token = "..."
```

### 字段说明

| 字段 | 含义 |
|---|---|
| `threads_num` | 图片下载/上传并发基数。建议从 1–3 开始。 |
| `interval` | 自动扫描间隔，例如 `1h`、`30m`。 |
| `search_params` | 传给 E-Hentai 搜索页的参数列表。 |
| `search_count` | 每次扫描最多处理的画廊数；`0` 表示关闭自动扫描。 |
| `trans_file` | EhTagTranslation 的 `db.text.json` 路径。 |
| `channel_id` | 发布频道，可为 `@channel_username` 或 Telegram 数字 ID。 |
| `group_id` | 讨论组数字 ID；用于管理员识别和评分互动。 |
| `auth_group_id` | 使用审核入群链接时要自动批准成员的群组 ID；不用该功能时仍填有效数字 ID。 |
| `kvault.base_url` | K-Vault 根地址，不要以 `/` 结尾。 |

## Telegram 命令

### 公共命令

| 命令 | 用途 |
|---|---|
| `/ping` | 检查 Bot 是否在线。 |
| `/help` | 查看命令说明。 |
| `/query <URL>` | 查询画廊收录状态和评分信息。 |
| `/best <天数起点> <天数终点>` | 查询指定时间区间的排行榜，例如 `/best 30 0`。 |
| `/random [标签] [数量]` | 从已收录画廊中随机检索，单次上限 10。 |
| `/fav` | 查看和管理个人收藏夹。 |
| `/challenge` | 发起画师猜谜。 |
| `/stats` | 查看收录和图片统计。 |
| `/upload <URL>` | 重新发布已收录画廊；不能用来让普通用户添加新画廊。 |

### 管理员命令

管理员身份以 `group_id` 对应群组中的 Telegram 管理员权限判定。

| 命令 | 用途 |
|---|---|
| `/upload <URL>` | 上传新的 E-Hentai / ExHentai 画廊。 |
| `/update <URL>` | 更新画廊元数据；也可回复频道转发消息执行。 |
| `/repair` | 补全当前回复消息对应画廊的缺页；不回复消息时扫描并修复全部不完整画廊。 |
| `/delete` | 回复频道转发消息后软删除画廊记录。 |
| `/erase` | 回复频道转发消息后删除画廊及对应消息记录。 |
| `/recheck` | 回复频道转发消息后检查图片和 Telegraph 预览。 |

## 数据与备份

部署目录中的持久化文件：

| 文件 | 用途 |
|---|---|
| `config.toml` | 运行配置和敏感凭据。 |
| `db.sqlite` | SQLite 主数据库。 |
| `db.sqlite-wal` / `db.sqlite-shm` | SQLite WAL 临时文件；运行中可能存在。 |
| `db.text.json` | 标签翻译数据库。 |

升级、迁移、硬删除或手工处理数据库前，先停止容器并备份数据库及 WAL 文件：

```sh
docker compose stop
cp -a db.sqlite db.sqlite-wal db.sqlite-shm backups/ 2>/dev/null || true
docker compose up -d
```

不要把 `config.toml`、数据库文件或 Cookie 上传到 Issue、日志平台或 Git 仓库。

## 升级

```sh
docker compose pull
docker compose up -d --force-recreate --remove-orphans
docker compose ps
```

升级后通过 `/ping` 和容器日志确认服务状态。若升级后异常，先使用备份的数据库和已知可用镜像 tag 回退，不要删除现有 `db.sqlite`。

## 从源码构建

仅适用于开发环境。需要 Rust 工具链和系统构建依赖。

```sh
git clone https://github.com/aiza2a/exloli-img.git
cd exloli-img
cp config.toml.example config.toml
cargo run --bin exloli
```

运行前必须完成配置；不要提交本地 `config.toml`。

## 开发与 CI

GitHub Actions 在 GitHub runner 上执行格式检查、Clippy、单元测试、发布构建与 Docker 镜像构建；不会在使用者部署服务器上自动运行这些检查。

本仓库的发布镜像由标签推送触发。发布前建议创建版本标签，例如：

```sh
git tag v0.4.2
git push origin v0.4.2
```

## 许可证

本项目采用 [MIT License](LICENSE)。
