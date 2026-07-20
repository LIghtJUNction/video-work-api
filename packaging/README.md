# Packaging · 打包分发

AUR **VCS** 包：`video-work-api-git`（从 GitHub 最新 HEAD 构建，`-git` 后缀）。

路径约定（与 `AGENTS.md` 一致）：

| 用途 | 路径 |
|------|------|
| 应用 | `/usr/lib/video-work-api` |
| 配置 | `/etc/video-work-api/config.env` |
| 数据 | `/var/lib/video-work-api` |
| CLI | `/usr/bin/vwactl` |
| 单元 | `video-work-api.service`（**不会**自动 enable） |

## 目录

```
packaging/aur/video-work-api-git/
  PKGBUILD
  video-work-api-git.install
  .SRCINFO
```

`PKGBUILD` 的 `source` 为：

```text
git+https://github.com/LIghtJUNction/video-work-api.git
```

`prepare()` 会 `git submodule update --init --recursive` 拉入 CosyVoice / FunClip。

## 本机用 AUR helper 部署

包提交到 AUR 之后：

```bash
paru -S video-work-api-git
# 或
yay -S video-work-api-git
```

首次：

```bash
sudo vwactl setup
sudo vwactl init
sudo vwactl model download   # 需明确同意的大体积下载
# 可选：编辑 /etc/video-work-api/config.env 写入 VWA_MCP_TOKEN
sudo systemctl start video-work-api.service
```

## 本地试构建（不上传 AUR）

```bash
cd packaging/aur/video-work-api-git
# 若本仓库尚未 push，可临时把 PKGBUILD source 改成 file:// 或本地 git URL
makepkg -si
```

刷新 `.SRCINFO`：

```bash
cd packaging/aur/video-work-api-git
makepkg --printsrcinfo > .SRCINFO
```

## 发布到 AUR

1. 在 [AUR](https://aur.archlinux.org) 创建 `video-work-api-git`（需 SSH 密钥）。
2. 克隆 AUR 仓库并同步本目录内容：

```bash
git clone ssh://aur@aur.archlinux.org/video-work-api-git.git
cp packaging/aur/video-work-api-git/{PKGBUILD,video-work-api-git.install,.SRCINFO} \
  video-work-api-git/
cd video-work-api-git
git add PKGBUILD video-work-api-git.install .SRCINFO
git commit -m "video-work-api-git: initial import"
git push
```

之后本仓库改动推送到 GitHub 后，用户 `paru -Syu video-work-api-git` 会重新 `git clone` 构建。仅当 `PKGBUILD` / `.install` 本身变更时才需要再 push AUR 仓库。

## 约束

- 安装钩子**不会** `systemctl enable`、不会改防火墙、不会自动下模型。
- 勿把参考音、权重、token、`.env`、SQLite 打进包。
- 升级后如依赖变化，执行 `sudo vwactl setup` 重建数据目录下的 venv。
