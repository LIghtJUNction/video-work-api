# 视频工作 API（Video Work API）

面向 **AI Agent** 的本地视频/语音工具集：授权零样本声音克隆（CosyVoice3）与
精确时间轴视频字幕提取（FunClip），通过 HTTP REST 与 **HTTP MCP**（`POST /mcp`）
对外提供能力。

- 二进制 / CLI：`vwactl` · crate：`video-work-api` · 默认端口：`7860`
- 实现语言：**Rust**（`src/` 布局）；仅 CosyVoice / FunClip 推理仍通过 Python 子进程
- 声音模型：`FunAudioLLM/Fun-CosyVoice3-0.5B-2512`（CosyVoice3，不是 Qwen3-TTS）
- 字幕：[`FunClip`](https://github.com/modelscope/FunClip) stage-1（FunASR）

> 仅可在获得说话人明确授权时克隆声音。详见 [SECURITY.md](SECURITY.md)。

## 安装

```bash
git clone --recurse-submodules <仓库地址>
cd video-work-api
cargo build --release
./scripts/vwactl setup
./scripts/vwactl init
./scripts/vwactl model download
export VWA_MCP_TOKEN="$(openssl rand -hex 32)"
./scripts/vwactl serve
```

打开 `http://127.0.0.1:7860`，输入 `vwactl init` 显示的一次性令牌并设置至少
12 位管理员密码。环境变量前缀为 `VWA_`，见 `config.env.example`。

## 能力概览

1. **声音克隆**：参考音 + 逐字稿 + 权利确认 → 生成任意目标文案语音。
2. **视频字幕**：视频放入 `VWA_VIDEO_INPUT_DIR`，调用
   `POST /api/videos/subtitles` 或 MCP `extract_video_subtitles`，返回带起止时间的
   segments 与 SRT。
3. **MCP 工具集**：`get_status`、`list_speakers`、`create_speaker`、
   `delete_speaker`、`add_voice_profile`、`delete_voice_profile`、
   `generate_speech`、`get_generation`、`extract_video_subtitles`。

```bash
curl -sS -X POST "http://127.0.0.1:7860/mcp" \
  -H "Authorization: Bearer $VWA_MCP_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

批量导入参考音：

```bash
vwactl import ./voices --confirm-rights
```

## systemd

软件包路径：`/usr/lib/video-work-api`、`/etc/video-work-api/config.env`、
`/var/lib/video-work-api`。单元不会被自动 enable。

```bash
sudo systemctl start video-work-api.service
```

如需 HTTPS，建议在反向代理终止 TLS（本服务校验 `VWA_SSL_*` 路径安全，但当前
构建以 HTTP 服务为主）。

## 开发

```bash
cargo test
cargo build --release
bash -n scripts/vwactl .agents/skills/video-work-api/scripts/health-check.sh
```
