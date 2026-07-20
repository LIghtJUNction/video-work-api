# Video Work API · 视频工作 API

面向 **AI Agent** 的本地化**工具集**：授权零样本声音克隆（CosyVoice3）与精确视频字幕提取（FunClip），提供经过认证的 HTTP REST API 以及 **HTTP MCP** 服务。

- 产品名称：**视频工作 API**
- 命令行工具 / 二进制：`vwactl` · Rust Crate：`video-work-api` · 默认端口：`7860`
- 技术栈：**Rust** 实现（`src/` 布局），仅 CosyVoice 和 FunClip 模型推理使用 Python 脚本子进程调用
- 声音克隆：Apache-2.0 授权的 [`FunAudioLLM/Fun-CosyVoice3-0.5B-2512`](https://huggingface.co/FunAudioLLM/Fun-CosyVoice3-0.5B-2512) (版本号 `29e01c4e8d000f4bcd70751be16fa94bf3d85a18`)，包含内置的 [`FunAudioLLM/CosyVoice`](https://github.com/FunAudioLLM/CosyVoice)
- 视频字幕：基于 [`modelscope/FunClip`](https://github.com/modelscope/FunClip) 提供的 stage-1 ASR (FunASR)，可提取带高精度时间戳的 SRT 段落

> 仅可在获得说话人明确授权时克隆声音。声音克隆技术可能会被用于身份冒充或欺诈。请务必阅读 [SECURITY.md](SECURITY.md)。

## 环境要求

- 运行 Linux 操作系统，并已安装 **Rust 1.75+** (`cargo`), Python 3.10, `uv`, FFmpeg, SoX, Git LFS 以及 Hugging Face CLI (`hf`，例如 `python-huggingface-hub`)
- 推荐使用 NVIDIA CUDA 以获得更快的 CosyVoice 推理速度；虽然也支持 CPU 推理，但速度较慢
- CosyVoice3 模型和 Python 依赖环境大约需要 10 GB 磁盘空间 (`vwactl model download` 会调用 `hf download` 并复用 Hub 缓存)
- 首次提取字幕时会自动下载 FunASR 模型

## 从源码安装与运行

```bash
git clone --recurse-submodules <你的仓库或Fork分支地址>
cd video-work-api
cargo build --release
./scripts/vwactl setup          # 仅为 CosyVoice/FunClip 初始化 Python 虚拟环境
./scripts/vwactl init
./scripts/vwactl model download
export VWA_MCP_TOKEN="$(openssl rand -hex 32)"
./scripts/vwactl serve
```

`vwactl init` 会在控制台打印一次性初始化令牌。在浏览器中打开 `http://127.0.0.1:7860`，输入该令牌并设置管理员密码，即可通过 Web 界面或 API/MCP 管理声音文件。

### 环境变量 (前缀 `VWA_`)

- `VWA_DATA_DIR`：数据存放根目录（默认 `~/.local/share/video-work-api`）
- `VWA_MODEL_DIR`：模型存放目录
- `VWA_COSYVOICE_ROOT`：CosyVoice 源码路径
- `VWA_FUNCLIP_ROOT`：FunClip 源码路径
- `VWA_SETUP_TOKEN_FILE`：初始化令牌保存文件路径
- `VWA_HOST`：监听的主机地址
- `VWA_PORT`：监听的端口号（默认 `7860`）
- `VWA_MCP_TOKEN`：MCP 服务的 Bearer Token 鉴权令牌
- `VWA_VIDEO_INPUT_DIR`：视频输入目录
- `VWA_REFERENCE_INPUT_DIR`：声音克隆参考音频目录
- `VWA_SUBTITLE_TIMEOUT`：字幕提取超时时间
- `VWA_PYTHON`：使用的 Python 可执行文件路径
- `VWA_PROJECT_ROOT`：项目根路径
- `VWA_SSL_CERTFILE` / `VWA_SSL_KEYFILE`：SSL 证书与密钥文件路径（可选，且会进行路径校验；生产环境推荐在反向代理中配置 HTTPS）

具体默认值可查看 `config.env.example`。

## 功能介绍

### 1. 声音克隆 (CosyVoice3)

上传或导入一段 5–30 秒的参考音频，需提供**精确**的逐字稿以及明确的权利确认。之后即可针对任意目标文本生成克隆语音（支持语速调节 0.75–1.25）。

批量导入声音：
```bash
vwactl import ./voices --confirm-rights
```

### 2. 视频字幕提取 (FunClip)

将视频放置于 `VWA_VIDEO_INPUT_DIR` 目录中（默认为 `$VWA_DATA_DIR/videos`），然后调用：

```bash
curl -sS -X POST "http://127.0.0.1:7860/api/videos/subtitles" \
  -H "Origin: http://127.0.0.1:7860" \
  -H "Content-Type: application/json" \
  -b cookies.txt \
  -d '{"video_path":"clip.mp4"}'
```

返回的响应将包含 `segments[{index,start,end,text}]` 数组以及完整的 `srt` 格式文本。

### 3. 面向 AI Agent 的 HTTP MCP

```bash
curl -sS -X POST "http://127.0.0.1:7860/mcp" \
  -H "Authorization: Bearer $VWA_MCP_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

提供的主要 MCP 工具：
- `get_status`：获取服务、模型和 FunClip 的就绪状态
- `list_speakers`：列出所有说话人及声音配置 (Profiles)
- `create_speaker`：创建说话人
- `delete_speaker`：删除说话人
- `add_voice_profile`：添加声音配置
- `delete_voice_profile`：删除声音配置
- `generate_speech`：生成克隆语音
- `get_generation`：获取已生成的音频信息
- `extract_video_subtitles`：提取视频字幕

MCP 客户端配置示例 (Cline / Claude Desktop 等)：
```json
{
  "mcpServers": {
    "video-work-api": {
      "url": "http://127.0.0.1:7860/mcp",
      "headers": { "Authorization": "Bearer ${VWA_MCP_TOKEN}" }
    }
  }
}
```

## systemd 服务配置

系统预设路径：
- 安装路径：`/usr/lib/video-work-api`
- 配置文件：`/etc/video-work-api/config.env`
- 数据目录：`/var/lib/video-work-api`

服务单元已随软件包安装，但默认**不会**自动启动或开机自启：

```bash
sudo systemctl start video-work-api.service
sudo systemctl stop video-work-api.service
```

## 开发与测试

```bash
cargo test
cargo build --release
bash -n scripts/vwactl .agents/skills/video-work-api/scripts/health-check.sh
```

### 目录结构

```
src/                 # Rust 库以及 vwactl 二进制入口
  main.rs            # 命令行工具 (init/setup/serve 等命令实现)
  lib.rs             # 库模块：config, database, studio, http, mcp 等
scripts/             # vwactl 包装脚本和 CosyVoice/FunClip 推理辅助脚本
static/              # Web UI 前端静态文件
vendor/              # CosyVoice 与 FunClip 的 git 子模块
```

*注意：模型权重、参考声音、生成的音频文件、SQLite 数据库、各类 Token 和配置文件等敏感/临时内容均已在 Git 中忽略。*
