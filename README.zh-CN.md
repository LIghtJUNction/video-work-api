<div align="center">

# Video Work API · 视频工作 API

**为用户、应用程序与 AI Agent 打造的自托管声音与字幕工作台。**

在一个经过认证的 Rust 服务中，使用 CosyVoice3 进行授权零样本声音克隆，
使用 FunClip 提取带时间码的视频字幕，并通过浏览器工作台完成日常操作。

[English](README.md) · [简体中文](README.zh-CN.md)

[![许可证：Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-9b4f3b.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/Rust-1.88%2B-202421.svg?logo=rust)](Cargo.toml)
[![平台：Linux](https://img.shields.io/badge/platform-Linux-d8dfcf.svg?logo=linux&logoColor=202421)](#环境要求)
[![AUR：video-work-api-git](https://img.shields.io/aur/version/video-work-api-git?label=AUR&color=c9775e)](https://aur.archlinux.org/packages/video-work-api-git)
[![MCP：HTTP](https://img.shields.io/badge/MCP-HTTP-62675f.svg)](#rest-api-与-http-mcp)

[快速开始](#快速开始) · [项目亮点](#为什么选择-video-work-api) · [MCP 工具](#video_editor-mcp-工具) · [安全](#安全与负责任使用) · [参与贡献](CONTRIBUTING.md)

</div>

![Video Work API 工作流：授权参考声音和沙箱视频输入经过 Rust 服务，连接 CosyVoice3、FunClip、浏览器工作台、REST API 与 HTTP MCP。](static/assets/video-work-api-overview.svg)

> [!IMPORTANT]
> 只有在可识别说话人给予明确、知情许可后，才能克隆或发布其声音。
> 声音克隆可能被用于冒充与欺诈。导入音频前请阅读
> [安全与负责任使用政策](SECURITY.md)。

## 为什么选择 Video Work API？

- **一个私有工作区，三种使用方式。** 你可以使用中英双语 Web 工作台、
  集成基于管理员会话认证的 REST API，或向可信 AI Agent 提供一个整合后的
  Bearer Token 认证 `video_editor` MCP 工具。
- **把参考声音变成可复用资产。** 按说话人和风格整理音频，随资料库成长
  重命名项目，并通过一行或多行文案生成独立 WAV 文件。
- **在同一服务中把视频转换为可用 SRT。** 提交沙箱目录路径或浏览器上传，
  获得带时间码的片段和可下载 SRT 文件。
- **围绕授权与本地所有权设计。** 精确逐字稿、明确权利确认、私有 Token 和
  路径沙箱由服务端执行，而不是只依赖客户端约定。
- **没有 Agent 也很好用。** 它首先是一个实用的本地工作台；MCP 提供自动化，
  但不会取代浏览器工作流。

## 可以用它做什么？

| 使用场景 | Video Work API 提供的能力 |
|---|---|
| 获得授权的创作者配音 | 复用不同说话风格，生成可下载 WAV 文件 |
| 本地化与无障碍内容 | 在本地生成旁白草稿和带时间码的 SRT 字幕 |
| 私有媒体工作流 | 将参考声音、输出、元数据和模型文件保留在自己的机器上 |
| Agent 辅助制作 | 让 Codex 或其他 MCP 客户端列出音色、生成语音和提取字幕 |
| 小团队声音资料库 | 在中英双语、密码保护的浏览器工作台中管理说话人与音色 |

项目有意保持明确边界：它不负责视频剪辑、字幕翻译、FunClip stage-2
裁剪，也不会替你判断是否已经获得合法授权。

## 工作原理

Rust 服务负责认证、元数据、授权检查、文件系统边界与结果发布。Python
辅助程序只用于内置 CosyVoice 和 FunClip 推理运行时。

1. 添加一段获得授权的 5–30 秒参考音频及其精确逐字稿，或在允许的输入
   边界中放置/上传视频。
2. CosyVoice3 使用选中的音色生成语音；FunClip **仅执行 stage-1 ASR**，
   输出带时间码的字幕片段和 SRT。
3. 通过浏览器工作台、经过认证的 REST API 或 HTTP MCP 使用结果。Agent
   得到的是生成音频的本地路径，而不是 base64 内容。

## 快速开始

### Arch Linux（AUR）

[`video-work-api-git`](https://aur.archlinux.org/packages/video-work-api-git)
会构建当前 Git 版本并安装服务目录。

```bash
paru -S video-work-api-git
sudo vwactl setup
sudo vwactl init
sudo vwactl model download   # 主动选择，约需 10 GB 网络流量与磁盘空间
sudo systemctl start video-work-api.service
```

打开 `http://localhost:7860`，使用 `sudo vwactl init` 打印的一次性 Token
完成设置。软件包会安装 systemd 单元，但**不会自动启用或启动服务**，也不会
修改防火墙规则。

### 从源码构建

```bash
git clone --recurse-submodules https://github.com/LIghtJUNction/video-work-api.git
cd video-work-api
cargo build --release
./scripts/vwactl setup
./scripts/vwactl init
./scripts/vwactl model download   # 主动选择，约需 10 GB 网络流量与磁盘空间
./scripts/vwactl serve
```

随后打开 `http://localhost:7860`。`setup` 会为推理运行时创建 Python
虚拟环境；API 服务本身是 Rust 编写的 `vwactl` 二进制文件。

<details>
<summary><strong>首次登录、模型下载与 Passkey</strong></summary>

`vwactl init` 会创建私有数据目录、SQLite 状态、权限为 0600 的持久 MCP
Token，以及一次性网页设置 Token。使用设置 Token 创建至少 12 位的管理员
密码。模型下载固定为下方列出的仓库与 Revision，会复用 Hugging Face Hub
缓存，并要求安装 `hf` CLI。

使用密码登录后可以注册 Passkey。WebAuthn 要求 HTTPS 域名；本地开发时
`http://localhost:<端口>` 例外，但不支持 IP 字面量来源。管理员密码会保留为
恢复登录方式。可通过 `./scripts/vwactl passwd`（源码安装）或
`sudo vwactl passwd`（软件包安装）交互式重置。

</details>

## 浏览器工作台

中英双语工作台提供：

- 一次性初始化、管理员会话与可选 Passkey 登录；
- 新建、重命名说话人和音色，以及带约束的安全删除；
- 浏览器麦克风录制或音频上传，并要求精确逐字稿和明确授权确认；
- 批量生成语音（每个非空行生成一个 WAV）、试听、失败重试与易读文件名下载；
- 从沙箱路径与本地上传批量提取字幕（每个文件最大 2 GiB），支持进度、
  重试、预览和 SRT 下载；
- 认证后的模型下载状态，以及管理员登录后用于配置 Codex 或 Claude Code 的
  **复制 Agent 提示词**流程，登录前不会暴露 Token。

浏览器麦克风采集要求安全上下文：HTTPS 或 localhost。

## REST API 与 HTTP MCP

它们是刻意分离的两条信任路径：

| 接口 | 目标客户端 | 认证方式 | 浏览器来源策略 |
|---|---|---|---|
| `/api/*` 下的 REST | Web 工作台和应用程序集成 | 状态、一次性设置、密码登录和 Passkey 登录入口按设计公开；认证端点使用不透明的 `HttpOnly`、`SameSite=Strict` 管理员会话 | 非安全请求要求 `Origin` 的主机与端口匹配 `Host` |
| `POST /mcp` 的 HTTP MCP | 可信 AI Agent | `Authorization: Bearer <VWA_MCP_TOKEN>` | Bearer 认证通过的 MCP 不受浏览器同源检查限制 |

REST 涵盖初始化、登录/退出、Passkey、模型下载、说话人/音色管理、语音生成、
经过认证的 WAV 获取和字幕提取。启动服务后可访问实时 `/docs` 页面查看精简
端点参考。

### `video_editor` MCP 工具

| 工具 | 用途 |
|---|---|
| `video_editor` | 列出/创建工程、浏览并按 revision 编辑 `project.vpe`、规划抽帧/封面、计算安全切点、运行质量门禁、管理凭证与生命周期、分配变体 ID，以及导出/查询/取消队列任务 |

管理员登录后可访问生产版 `/editor`。界面采用已批准的原生代码工作台骨架与实时
工程检查器：多个虚拟工程根、可展开目录树、标签页、带行号的纯文本编辑、解析后的
时间轴/marker/transition/variant/gate 以及脱敏渲染队列保持同屏。
`project.vpe` 是唯一可写文件；保存携带 `expected_revision`，远端更新绝不会覆盖
本地脏缓冲区。认证且同源的 `GET /api/editor/events` SSE 会发送初始工程/任务状态
及脱敏 revision/job 变化，不接受 Bearer/query token，也不暴露私有路径；重连期间
界面回退到 `get_job` 轮询。

每个工程都是虚拟文件夹，唯一可写文件是 `project.vpe`；生成的
`.history/`、`receipts/`、`exports/` 可浏览但只读。写入必须携带
`expected_revision`；内容先写入私有临时文件并 `fsync`，再登记 durable write
intent，随后原子发布。进程中断后，会在工程锁内从通过 SHA-256 校验的 staged、
history 或 current 副本向前恢复。history 采用 create-only 的 hash 命名文件，
只有 history 与 `project.vpe` 均落盘后 DB revision 才会推进。只有当前且已校验的
revision 可以导出。

同一工具还提供真实排队执行的 `extract_analysis_frames`、
`analyze_safe_trims` 与 `render_cover`。它们共用持久 FIFO 单 Worker 队列、
固定服务端 FFmpeg/FFprobe、私有输入快照、取消、超时和进程组恢复。抽帧默认
12 帧、360x640，所有时间戳严格早于 EOF，并生成带白色时间戳/分段 ASR
元数据栏的图片；不执行 VLM 分析。安全切点从真实音轨检测静音，再调用保守切点
算法；未提供词级时间戳且已配置 FunClip 时，会在冻结输入上运行固定 stage-1
ASR，严格解析真实 token 时间戳，绝不插值。字幕提取同时返回分段、SRT 与词级
时间戳。封面操作必须指定当前 VPE 声明的稳定 variant key，并绑定当前工程
revision、document、VariantSpec 与 CoverSpec；两张图片实际解码通过后才发布
`<variant-key>-cover-original.png` 与 `<variant-key>.jpg`。pre-package 只接受
按 variant key 提交的 succeeded cover job ID，不接受调用方自述路径或哈希。
渲染器只能写入私有 `attempt-1/` 目录。校验完成后，Worker 会持久登记有序
publication intent，绑定每个源/目标相对路径、大小与 SHA-256；随后通过固定的
工程/exports 目录描述符、已 fsync 的临时文件和 no-replace rename 发布。重启时，
已有 intent 会直接向前完成而不重新渲染；没有 intent 的残留 attempt 会先安全
删除再重新排队。取消与发布通过同一个数据库 CAS 决胜：取消先落库时不会发布
任何文件并进入 cleanup-pending；intent 先落库时后续取消为 no-op，发布继续完成。
目标已有相同内容时视为成功；内容冲突时保留既有 exports，
任务保持 running 且标记 publication-blocked，并停止后续 claim 供运维检查。
封面双文件、视频多文件与报告均使用同一协议；只有数据库提交 succeeded 后，
私有 attempt 才进入 cleanup-pending。
`get_job` 只返回公开任务状态与工程相对交付路径，不泄露私有快照、日志、计划绝对
路径、执行器参数或内部错误；失败只返回有限公开 code 与通用消息。任务终态会删除冻结源媒体及私有 Replay 媒体，同时保留规范
计划、已验证报告、日志和公开分析结果。
清理和归档默认 `dry_run: true`，只有显式传入 `dry_run: false` 且沙箱、
白名单和服务端门禁全部通过时才会修改文件。清理只接受服务定义的 `cache/`、
`proxies/`、`passes/` 路径，工程核心 `.tmp/` 永不清理。源素材归档会验证当前
revision 的完整凭证链，通过 durable journal/staging 只搬迁 VPE 声明的
`project/assets` 素材，并始终保留 exports。Master 与每个变体都必须具备有效
`ftyp` 且 `moov` 位于 `mdat` 前。pre-package 与 acceptance 只接受精确匹配当前
工程 revision 的 succeeded `video_project` 任务 ID；Worker 在重新计算主输出、
Replay、报告和每个变体的 index、语言、画幅、水印、CTA、路径及哈希后，才写入
数据库可信证明。调用方不能提交报告路径或自述 Replay。每个变体必须使用不同
路径和不同成片，copy/封面声明哈希均会按实际文件重算。不可变 PASS 报告与凭证按
`receipts/rev-<n>/<phase>/` 保存；签名绑定精确报告哈希、工程、revision、
document、门禁结果、输入输出、执行器和上一凭证。FAIL 或签名不可用都不会占用
canonical PASS 路径；配置 key 后可对同一 revision 重试。

全英文 VPE 文档描述 canvas、source、track、clip、cut、hold、transition、
marker、variant 与 gate：

```text
project "Aurora Launch" {
  canvas 1080x1920 @ 30fps
  source host = "assets/host.mp4"
  source detail = "assets/detail.mp4"
  timeline {
    track main primary {
      clip host source 00:00:00.000..00:00:03.120 at 00:00:00.000
      cut at 00:00:03.120
      transition cross_dissolve at 00:00:03.120 duration 00:00:00.200
      clip host source 00:00:03.120..00:00:06.200 at 00:00:03.120
    }
    track overlay product type broll {
      clip detail source 00:00:00.000..00:00:01.000 at 00:00:02.000
    }
    track overlay graphics type effect {
      effect vignette at 00:00:02.800..00:00:03.200
    }
  }
  marker "Opening hook" at 00:00:03.000
  variant "ZH-EN" aspect 9:16 subtitles "subs.zh-en.ass" watermark "brand/logo.png" cta "Learn more"
  gate pre_render require input_manifest, continuous_timeline, opening_hook, subtitle_overflow
  gate pre_package require output_specifications, cover_match, copy_consistency
  gate acceptance require deterministic_replay, faststart
}
```

`export` 会把当前已校验 revision 编译成不可变的 canonical EDL bundle。持久化
单并发 worker 会再次核验文档、渲染器与冻结工程素材哈希，先生成 `master.mp4`，
再派生所声明的画幅/字幕/水印/CTA 变体，并写入虚拟工程只读的 `exports/` 树。
命名主轨会按等时长图层合成并混音，变体使用稳定的序号/语言/画幅 key。未知
动效或转场、符号链接素材和过期 revision 都会 fail closed。
旧的 XRY 直接 REST/MCP 渲染提交入口已禁用。

管理员登录后，**复制 Agent 提示词**会提供 Codex 与 Claude Code 的安装说明。
项目级配置分别使用 Codex 的 `.codex/config.toml` 或 Claude Code 的
`.mcp.json`；用户/全局级配置使用各客户端的用户级 MCP 配置。生成的配置都
包含静态 Bearer Token，必须按秘密文件保护。

<details>
<summary><strong>MCP Token 生命周期</strong></summary>

Token 默认保存在 `$VWA_DATA_DIR/mcp-token`，重启和升级后保持不变；
`VWA_MCP_TOKEN_FILE` 可以更改路径，`VWA_MCP_TOKEN` 是优先级更高的兼容覆盖项。
如需轮换，请主动执行 `vwactl mcp-token rotate`，重启服务、登录、复制新的 Agent
提示词、替换客户端配置，然后重启或新建客户端会话并验证实时工具。仅重启
服务无法更新客户端中的静态请求头。

</details>

## 环境要求

- Linux、**Rust 1.88+**、Python 3.10+、`uv`、FFmpeg、SoX、Git LFS 与
  Hugging Face CLI（`hf`，例如由 `python-huggingface-hub` 提供）
- 推荐 NVIDIA CUDA 加速 CosyVoice 推理；CPU 可以运行，但速度较慢
- 固定 CosyVoice3 快照与运行环境约需 10 GB 网络流量和磁盘空间
- 首次提取字幕时会额外下载 FunASR 模型

### 固定模型与运行时范围

- 声音模型：[`FunAudioLLM/Fun-CosyVoice3-0.5B-2512`](https://huggingface.co/FunAudioLLM/Fun-CosyVoice3-0.5B-2512)
- Revision：`29e01c4e8d000f4bcd70751be16fa94bf3d85a18`
- 推理运行时：内置 [`FunAudioLLM/CosyVoice`](https://github.com/FunAudioLLM/CosyVoice)（CosyVoice3，不是 Qwen3-TTS）
- 字幕：内置 [`modelscope/FunClip`](https://github.com/modelscope/FunClip)，仅 stage-1 FunASR Paraformer

## 安全与负责任使用

- 针对声音及其预期用途获得明确、知情许可。
- 保留精确参考逐字稿，不得绕过 `confirm_rights`。
- 不要将声音、逐字稿、生成媒体、SQLite 数据、Token、凭据、模型权重、缓存
  和环境文件提交到 Git。
- MCP 参考音频和视频受各自配置的输入目录限制；符号链接与不安全路径会被拒绝。
- 默认监听 `0.0.0.0:7860`，局域网可见。面对不可信网络时，请绑定回环地址，
  或使用带认证的 HTTPS 反向代理与可信子网过滤。切勿直接暴露到公网。
- 安装过程不会启用服务，也不会修改防火墙规则。

完整威胁模型和报告渠道请阅读 [SECURITY.md](SECURITY.md)。

<details>
<summary><strong>配置与安装路径</strong></summary>

配置变量使用 `VWA_*` 前缀。常用设置包括 `VWA_DATA_DIR`、`VWA_MODEL_DIR`、
`VWA_COSYVOICE_ROOT`、`VWA_FUNCLIP_ROOT`、`VWA_HOST`、`VWA_PORT`、
`VWA_VIDEO_INPUT_DIR`、`VWA_REFERENCE_INPUT_DIR`、`VWA_MCP_TOKEN_FILE`、
`VWA_VIDEO_PROJECTS_DIR`、
`VWA_RECEIPT_KEY_FILE`、
`VWA_XRY_TASK_ROOT`、`VWA_XRY_SOURCE_ROOT`、`VWA_XRY_RENDERER`、`VWA_XRY_PYTHON`、
`VWA_RENDER_TIMEOUT`，
以及可选的 `VWA_SSL_CERTFILE` / `VWA_SSL_KEYFILE`。证书和密钥路径只会按成对、
常规非符号链接文件进行校验，并不会启用内置 HTTPS；服务仍通过 HTTP 提供。
如需非 localhost 或局域网浏览器安全上下文，请使用 HTTPS 反向代理。
权威默认值请查看
[`config.env.example`](config.env.example)。

软件包安装路径：

| 用途 | 路径 |
|---|---|
| 应用程序 | `/usr/lib/video-work-api` |
| 配置 | `/etc/video-work-api/config.env` |
| 私有数据 | `/var/lib/video-work-api` |
| 服务 | `video-work-api.service` |

源码安装时使用 `vwactl paths` 和 `vwactl status` 查看生效配置。对软件包安装的
单元显式执行 `systemctl start`、`stop` 或 `restart`；不要假定它已经启用。

XRY 渲染必须通过专用只读组部署 receipt key，不要把服务加入宽泛的运维组：

```bash
sudo groupadd --system --force xry-render
sudo usermod --append --groups xry-render video-work-api
sudo usermod --append --groups xry-render xry
sudo python /srv/xry/.agents/skills/xry-video-acceptance/scripts/generate_receipt_key.py
sudo chown root:xry-render /etc/xry/render-receipt.hmac.key
sudo chmod 0640 /etc/xry/render-receipt.hmac.key
```

软件包单元声明 `SupplementaryGroups=xry-render`；安装钩子会创建该组，将
`video-work-api` 和已存在的 `xry` 账号加入组，并把已有 key 规范为
`root:xry-render`、`0640`。钩子不会 enable 或启动服务。

Video Project 导出进入持久 FIFO 队列。数据库约束和独占 worker lease 保证跨进程
全局最多只有一个运行任务。取消、超时、渲染器错误和服务关机都会终止整个
渲染器进程组并释放槽位。FIFO 使用持久化单调入队序号，不依赖墙钟时间或
UUID。每个任务持有绑定工程、revision 与文档哈希的 canonical EDL 和冻结素材
快照；独立二次渲染的所有输出必须逐字节哈希一致。异常重启后，durable launch
handshake 会覆盖 spawn 到数据库写 PID 的窗口。PID/starttime 落库后，即使
handshake 丢失或损坏，也会独立核验 `/proc` 进程组与固定执行器/job cmdline。
发信号前先持久化 recovery intent；身份不明确时任务保持 `running` 并阻止继续
取任务。终态清理同样用 cleanup-pending 标志持续重试至完成。

</details>

## 项目结构

```text
src/                 Rust 库与 vwactl 服务/CLI
scripts/             安装包装器与推理辅助程序
static/              中英双语浏览器工作台与静态资源
vendor/              CosyVoice 与 FunClip 子模块
systemd/             软件包服务单元
packaging/aur/       AUR VCS 软件包文件
tests/               API、CLI 与浏览器契约测试
```

## 文档与开发

- [安全与负责任使用](SECURITY.md)
- [配置示例](config.env.example)
- [打包与 AUR 说明](packaging/README.md)
- [贡献指南](CONTRIBUTING.md)
- 实时 API 参考：运行中实例的 `/docs`

```bash
cargo test
cargo build --release
bash -n scripts/vwactl .agents/skills/video-work-api/scripts/health-check.sh
```

测试使用伪推理和临时目录，不需要下载模型。请保持英文与简体中文 README 的
章节同步。

## 许可证

Video Work API 使用 [Apache License 2.0](LICENSE)。内置模型代码与模型文件
仍分别遵循其上游许可证和使用条款。
