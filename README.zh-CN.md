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

[快速开始](#快速开始) · [项目亮点](#为什么选择-video-work-api) · [MCP 工具](#11-个-mcp-工具) · [安全](#安全与负责任使用) · [参与贡献](CONTRIBUTING.md)

</div>

![Video Work API 工作流：授权参考声音和沙箱视频输入经过 Rust 服务，连接 CosyVoice3、FunClip、浏览器工作台、REST API 与 HTTP MCP。](static/assets/video-work-api-overview.svg)

> [!IMPORTANT]
> 只有在可识别说话人给予明确、知情许可后，才能克隆或发布其声音。
> 声音克隆可能被用于冒充与欺诈。导入音频前请阅读
> [安全与负责任使用政策](SECURITY.md)。

## 为什么选择 Video Work API？

- **一个私有工作区，三种使用方式。** 你可以使用中英双语 Web 工作台、
  集成基于管理员会话认证的 REST API，或向可信 AI Agent 提供 11 个
  Bearer Token 认证的 MCP 工具。
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

### 11 个 MCP 工具

| 工具 | 用途 |
|---|---|
| `get_status` | 查看服务、模型、FunClip 与 MCP 就绪状态 |
| `list_speakers` | 列出说话人及其音色 |
| `create_speaker` | 新建说话人条目 |
| `rename_speaker` | 在保持名称唯一的前提下重命名说话人 |
| `delete_speaker` | 仅在音色已移除后删除说话人 |
| `add_voice_profile` | 使用精确逐字稿和 `confirm_rights=true` 导入沙箱内的参考音频 |
| `rename_voice_profile` | 在保持同一说话人下风格名唯一的前提下重命名音色 |
| `delete_voice_profile` | 删除没有生成历史的音色 |
| `generate_speech` | 生成 CosyVoice3 语音，返回生成 ID/本地音频路径 |
| `get_generation` | 查询生成状态与完成后的音频路径 |
| `extract_video_subtitles` | 使用 FunClip stage-1 ASR 提取带时间码的 SRT 数据 |

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
`VWA_VIDEO_INPUT_DIR`、`VWA_REFERENCE_INPUT_DIR`、`VWA_MCP_TOKEN_FILE`，
以及可选的 `VWA_SSL_CERTFILE` / `VWA_SSL_KEYFILE`。权威默认值请查看
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
