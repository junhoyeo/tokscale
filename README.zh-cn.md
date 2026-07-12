<!-- <CENTERED SECTION FOR GITHUB DISPLAY> -->

<div align="center">

[![Tokscale](./.github/assets/hero-v2.png)](https://tokscale.ai)

</div>

> 高性能 CLI 工具和可视化仪表板，用于跟踪多个平台上 AI 编程助手的 Token 使用量和成本。

> [!TIP]
>
> **v2 已发布 — 原生 Rust TUI、跨平台支持等。** <br />
> 我每周都会发布新的开源项目。不要错过下一个。
>
> | [<img alt="GitHub Follow" src="https://img.shields.io/github/followers/junhoyeo?style=flat-square&logo=github&labelColor=black&color=24292f" width="156px" />](https://github.com/junhoyeo) | 在 GitHub 上关注 [@junhoyeo](https://github.com/junhoyeo) 获取更多项目。涉及 AI、基础设施等各个领域。 |
> | :-----| :----- |
> [<img alt="Discord link" src="https://img.shields.io/discord/1480206352755458110?color=5865F2&label=discord&labelColor=black&logo=discord&logoColor=white&style=flat-square" width="156px" />](https://discord.gg/h6DUGWdBbm) | 加入我们的 [Discord](https://discord.gg/h6DUGWdBbm) — 与全球顶尖的开发者一起交流。 |

<div align="center">

[![GitHub Release](https://img.shields.io/github/v/release/junhoyeo/tokscale?color=0073FF&labelColor=black&logo=github&style=flat-square)](https://github.com/junhoyeo/tokscale/releases)
[![npm Version](https://img.shields.io/npm/v/tokscale?color=0073FF&labelColor=black&style=flat-square&logo=npm)](https://www.npmjs.com/package/tokscale)
[![npm Downloads](https://img.shields.io/npm/dt/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://www.npmjs.com/package/tokscale)
[![GitHub Contributors](https://img.shields.io/github/contributors/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/graphs/contributors)
[![GitHub Forks](https://img.shields.io/github/forks/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/network/members)
[![GitHub Stars](https://img.shields.io/github/stars/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/stargazers)
[![GitHub Issues](https://img.shields.io/github/issues/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/issues)
[![License](https://img.shields.io/badge/license-MIT-white?labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/blob/master/LICENSE)
[![Coverage](https://raw.githubusercontent.com/junhoyeo/tokscale/refs/heads/main/.github/badges/coverage.svg)](https://github.com/junhoyeo/tokscale/issues/403)

[🇺🇸 English](README.md) | [🇰🇷 한국어](README.ko.md) | [🇯🇵 日本語](README.ja.md) | [🇨🇳 简体中文](README.zh-cn.md)

</div>

<!-- </CENTERED SECTION FOR GITHUB DISPLAY> -->

| Overview | Models |
|:---:|:---:|
| ![TUI Overview](.github/assets/tui-overview.png) | ![TUI Models](.github/assets/tui-models.png) | 

| Daily Summary | Stats |
|:---:|:---:|
| ![TUI Daily Summary](.github/assets/tui-daily.png) | ![TUI Stats](.github/assets/tui-stats.png) | 

| Frontend (3D Contributions Graph) | Wrapped 2025 |
|:---:|:---:|
| <a href="https://tokscale.ai"><img alt="Frontend (3D Contributions Graph)" src=".github/assets/frontend-contributions-graph.png" width="700px" /></a> | <a href="#wrapped-2025"><img alt="Wrapped 2025" src=".github/assets/wrapped-2025-agents.png" width="700px" /></a> |

> **运行 [`bunx tokscale submit`](#社交平台命令) 将您的使用数据提交到排行榜并创建公开个人资料！**

## 概述

**Tokscale** 帮助您监控和分析以下平台的 Token 消耗：

| 图标 | 客户端 | 数据位置 |
|------|----------|---------------|
| <img width="48px" src=".github/assets/client-opencode.png" alt="OpenCode" /> | [OpenCode](https://github.com/sst/opencode) | `~/.local/share/opencode/opencode.db` (1.2+，包含 `opencode-stable.db` 等所有渠道) 或 `~/.local/share/opencode/storage/message/` |
| <img width="48px" src=".github/assets/client-claude.jpg" alt="Claude" /> | [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `~/.claude/projects/` 和 `~/.claude/transcripts/` |
| <img width="48px" src=".github/assets/client-openclaw.jpg" alt="OpenClaw" /> | [OpenClaw](https://openclaw.ai/) | `~/.openclaw/agents/` (+ 旧版: `.clawdbot`, `.moltbot`, `.moldbot`) |
| <img width="48px" src=".github/assets/client-openai.jpg" alt="Codex" /> | [Codex CLI](https://github.com/openai/codex) | `~/.codex/sessions/` |
| <img width="48px" src=".github/assets/client-sakana.png" alt="Sakana Fugu" /> | [Sakana Fugu](https://sakana.ai/fugu/) | 通过 Codex 追踪 — `~/.codex/sessions/*.jsonl` (`model_provider: sakana`) |
| <img width="48px" src=".github/assets/client-copilot.jpg" alt="Copilot" /> | [GitHub Copilot CLI](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/use-the-github-copilot-coding-agent-in-cli) | `~/.copilot/otel/*.jsonl` (+ `COPILOT_OTEL_FILE_EXPORTER_PATH`) |
| <img width="48px" src=".github/assets/client-hermes.png" alt="Hermes Agent" /> | [Hermes Agent](https://github.com/NousResearch/hermes-agent) | `$HERMES_HOME/state.db` 和 `$HERMES_HOME/profiles/*/state.db`（回退：`~/.hermes/...`） |
| <img width="48px" src=".github/assets/client-gemini.png" alt="Gemini" /> | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `$GEMINI_CLI_HOME/tmp/*/chats/*.json`（回退：`~/.gemini/tmp/*/chats/*.json`） |
| <img width="48px" src=".github/assets/client-cursor.jpg" alt="Cursor" /> | [Cursor IDE](https://cursor.com/) | Cursor API 导出缓存于 `~/.config/tokscale/cursor-cache/usage*.csv`（而非 `~/.cursor`） |
| <img width="48px" src=".github/assets/client-amp.png" alt="Amp" /> | [Amp (AmpCode)](https://ampcode.com/) | `~/.local/share/amp/threads/` |
| <img width="48px" src=".github/assets/client-codebuff.png" alt="Codebuff" /> | [Codebuff](https://codebuff.com/) | `~/.config/manicode/`（+ `manicode-dev`、`manicode-staging`；可通过 `CODEBUFF_DATA_DIR` 覆盖） |
| <img width="48px" src=".github/assets/client-droid.png" alt="Droid" /> | [Droid (Factory Droid)](https://factory.ai/) | `~/.factory/sessions/` |
| <img width="48px" src=".github/assets/client-pi.png" alt="Pi" /> | [Pi](https://github.com/badlogic/pi-mono) | `~/.pi/agent/sessions/` 和 `~/.omp/agent/sessions/`（[Oh My Pi](https://github.com/can1357/oh-my-pi)） |
| <img width="48px" src=".github/assets/client-kimi.png" alt="Kimi" /> | [Kimi CLI](https://github.com/MoonshotAI/kimi-cli) / [Kimi Code](https://github.com/MoonshotAI/kimi-code) | kimi-cli: `~/.kimi/sessions/` kimi-code: `~/.kimi-code/sessions/`（可通过 `KIMI_CODE_HOME` 覆盖） |
| <img width="48px" src=".github/assets/client-qwen.png" alt="Qwen" /> | [Qwen CLI](https://github.com/QwenLM/qwen-cli) | `~/.qwen/projects/` |
| <img width="48px" src=".github/assets/client-roocode.png" alt="Roo Code" /> | [Roo Code](https://github.com/RooCodeInc/Roo-Code) | `~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks/` (+ server: `~/.vscode-server/data/User/globalStorage/rooveterinaryinc.roo-cline/tasks/`) |
| <img width="48px" src=".github/assets/client-kilocode.png" alt="Kilo" /> | [Kilo](https://github.com/Kilo-Org/kilocode) | `~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/` (+ server: `~/.vscode-server/data/User/globalStorage/kilocode.kilo-code/tasks/`) |
| <img width="48px" src=".github/assets/client-kilocode.png" alt="Kilo CLI" /> | [Kilo CLI](https://github.com/nicepkg/kilo) | `~/.local/share/kilo/kilo.db` |
| <img width="48px" src=".github/assets/client-mux.png" alt="Mux" /> | [Mux](https://github.com/coder/mux) | `~/.mux/sessions/` |
| <img width="48px" src=".github/assets/client-crush.png" alt="Crush" /> | [Crush](https://crush.ai/) | `$XDG_DATA_HOME/crush/projects.json`（项目注册表；回退路径：`~/.local/share/crush/projects.json`） |
| <img width="48px" src=".github/assets/client-goose.png" alt="Goose" /> | [Goose](https://github.com/aaif-goose/goose) | `~/.local/share/goose/sessions/sessions.db`（+ macOS Application Support、旧版 Block/goose 路径；可通过 `GOOSE_PATH_ROOT` 覆盖） |
| <img width="48px" src=".github/assets/client-antigravity.png" alt="Antigravity" /> | [Google Antigravity](https://antigravity.google/) | 通过 `tokscale antigravity sync` 缓存到 `~/.config/tokscale/antigravity-cache/sessions/*.jsonl`（使用本地语言服务器 RPC） |
| <img width="48px" src=".github/assets/client-antigravity.png" alt="Antigravity CLI" /> | [Antigravity CLI](https://antigravity.google/) | `~/.gemini/antigravity-cli/conversations/*.db`（可通过 `GEMINI_CLI_HOME` 覆盖 Gemini 主目录；本地 SQLite，直接读取 — 无需 `antigravity sync`） |
| <img width="48px" src=".github/assets/client-trae.png" alt="Trae" /> | [Trae IDE](https://www.trae.ai/) / [Trae Solo](https://www.trae.ai/solo)（国际版） | 通过 `tokscale trae sync` 缓存到 `~/.config/tokscale/trae-cache/sessions/*.json`（来自官方 API 的账号级使用量） |
| <img width="48px" src="https://github.com/warpdotdev.png" alt="Warp" /> | [Warp](https://www.warp.dev/) / Oz | 通过 `tokscale warp sync` 缓存到 `~/.config/tokscale/warp-cache/usage.json`（仅汇总请求数和消费金额，不含 token 转录） |
| <img width="48px" src="https://github.com/xai-org.png" alt="Grok Build" /> | Grok Build | `$GROK_HOME/sessions/*/*/updates.jsonl`（回退：`~/.grok/sessions/*/*/updates.jsonl`） |
| <img width="48px" src=".github/assets/client-zed.webp" alt="Zed Agent" /> | [Zed Agent](https://zed.dev/docs/ai/agent-panel) | `~/.local/share/zed/threads/threads.db`（macOS: `~/Library/Application Support/Zed/threads/threads.db`；Windows: `%LOCALAPPDATA%/Zed/threads/threads.db`；仅限托管 Zed 模型，不含外部 ACP 代理） |
| <img width="48px" src="https://github.com/kirodotdev.png" alt="Kiro" /> | Kiro | `~/.kiro/sessions/cli/*.json`（+ `*.jsonl`）、`~/.local/share/kiro-cli/data.sqlite3`（macOS: `~/Library/Application Support/kiro-cli/data.sqlite3`），以及 Kiro IDE globalStorage 快照（`Kiro/User/globalStorage/kiro.kiroagent`；macOS Application Support、Linux `~/.config/Kiro`、Windows `%APPDATA%\Kiro`） |
| <img width="48px" src="https://github.com/cline.png" alt="Cline" /> | [Cline](https://github.com/cline/cline) | VS Code globalStorage 任务（Linux: `~/.config/Code/...`；macOS: `~/Library/Application Support/Code/...`；Windows: `%APPDATA%\Code\...`；server: `~/.vscode-server/data/User/globalStorage/saoudrizwan.claude-dev/tasks/`） |
| <img width="48px" src="https://github.com/user-attachments/assets/7246e920-f3f8-4b6e-847e-030ae04e86c2" alt="Gajae-Code" /> | [gajae-code (gjc)](https://github.com/Yeachan-Heo/gajae-code) | `~/.gjc/agent/sessions/`（可通过 `GJC_CODING_AGENT_DIR`、`GJC_CONFIG_DIR`、`PI_CONFIG_DIR` 覆盖；Linux/macOS 上 `$XDG_DATA_HOME/gjc/sessions/` 亦支持） |
| <img width="48px" src=".github/assets/client-jcode.png" alt="Jcode" /> | [Jcode](https://github.com/1jehuang/jcode) | `~/.jcode/sessions/session_*.json` + `session_*.journal.jsonl` sidecar（可通过 `JCODE_HOME` 覆盖） |
| <img width="48px" src="https://github.com/XiaomiMiMo.png" alt="MiMo Code" /> | [MiMo Code](https://github.com/XiaomiMiMo/MiMo-Code) | `~/.local/share/mimocode/mimocode.db`（XDG 数据目录；SQLite） |
| <img width="48px" src="https://github.com/JetBrains.png" alt="Junie" /> | [Junie](https://www.jetbrains.com/junie/) | `~/.junie/sessions/*/events.jsonl` |
| <img width="48px" src="https://raw.githubusercontent.com/CommandCodeAI/command-code/main/.github/commandcode/logo/command-code-logo-black-bg.png" alt="Command Code" /> | [Command Code](https://github.com/CommandCodeAI/command-code) | `~/.commandcode/projects/**/*.jsonl`（Token 使用量按 ~4 字符/Token 从转录估算；不会持久化到磁盘） |
| <img width="48px" src="https://github.com/zai-org.png" alt="ZCode" /> | [ZCode](https://zcode.z.ai/) | `~/.zcode/cli/db/db.sqlite`（v2 用量数据库）和 `~/.zcode/projects/**/*.jsonl`（旧版记录） |
| <img width="48px" src="https://github.com/alibaba.png" alt="OpenCodeReview" /> | [OpenCodeReview](https://github.com/alibaba/open-code-review) | `~/.opencodereview/sessions/**/*.jsonl` |
| <img width="48px" src="https://pc3.gtimg.com/softmgr/logo/48/43068_48_1764842447.png" alt="CodeBuddy" /> | [CodeBuddy](https://www.codebuddy.cn/docs/cli/overview)（CLI、IDE、VS Code 插件） | `~/.codebuddy/projects/**/*.jsonl` + 扩展日志 |
| <img width="48px" src="https://static.workbuddy.cn/web/agents/008054d6beaaf4a83e2d049e982e1244560726dc/assets/share-logo.png" alt="WorkBuddy" /> | WorkBuddy | `~/.workbuddy/projects/**/*.jsonl` + SQLite 回退 |
| <img width="48px" src=".github/assets/client-devin.jpg" alt="Devin CLI" /> | [Devin CLI](https://devin.ai/) | `~/.local/share/devin/cli/sessions.db`（SQLite） |
| <img width="48px" src=".github/assets/client-devin.jpg" alt="Devin Desktop" /> | [Devin Desktop](https://devin.ai/) | ACP 事件：macOS `~/Library/Application Support/Devin/User/acp-events/`；Linux `~/.config/Devin/User/acp-events/`；Windows `%APPDATA%\Devin\User\acp-events\` |
| <img width="48px" src=".github/assets/client-synthetic.png" alt="Synthetic" /> | [Synthetic](https://synthetic.new/) | 通过 `hf:` 模型前缀或 `synthetic` provider 从其他来源重归属（+ [Octofriend](https://github.com/synthetic-lab/octofriend): `~/.local/share/octofriend/sqlite.db`） |

使用 [🚅 LiteLLM 的价格数据](https://github.com/BerriAI/litellm)提供实时价格计算，支持分层定价模型和缓存 Token 折扣。

### 为什么叫 "Tokscale"？

这个项目的灵感来自 **[卡尔达肖夫指数(Kardashev Scale)](https://zh.wikipedia.org/wiki/%E5%8D%A1%E5%B0%94%E8%BE%BE%E8%82%96%E5%A4%AB%E6%8C%87%E6%95%B0)**，这是天体物理学家尼古拉·卡尔达肖夫提出的一种根据能源消耗来衡量文明技术发展水平的方法。I 型文明利用其行星上所有可用的能源，II 型文明捕获其恒星的全部输出，III 型文明则掌控整个星系的能源。

在 AI 辅助开发的时代，**Token 就是新的能源**。它们驱动我们的思考，提升我们的生产力，推动我们的创造性产出。正如卡尔达肖夫指数在宇宙尺度上追踪能源消耗，Tokscale 在您攀登 AI 增强开发的阶梯时测量您的 Token 消耗。无论您是休闲用户还是每天消耗数百万 Token，Tokscale 都能帮助您可视化从行星级开发者到银河级代码架构师的旅程。

## 目录

- [概述](#概述)
  - [为什么叫 "Tokscale"？](#为什么叫-tokscale)
- [功能](#功能)
- [安装](#安装)
  - [快速开始](#快速开始)
  - [先决条件](#先决条件)
  - [开发环境设置](#开发环境设置)
  - [构建原生模块](#构建原生模块)
- [使用方法](#使用方法)
  - [基本命令](#基本命令)
  - [TUI 功能](#tui-功能)
  - [按平台筛选](#按平台筛选)
  - [日期筛选](#日期筛选)
  - [价格查询](#价格查询)
  - [社交平台命令](#社交平台命令)
  - [Autosubmit](#autosubmit)
  - [Cursor IDE 命令](#cursor-ide-命令)
  - [Antigravity 命令](#antigravity-命令)
  - [Trae 命令](#trae-命令)
  - [Warp/Oz 命令](#warpoz-命令)
  - [任务归因报告](#任务归因报告)
  - [订阅使用量](#订阅使用量)
  - [示例输出](#示例输出--light-版本)
  - [配置](#配置)
  - [环境变量](#环境变量)
- [前端可视化](#前端可视化)
  - [功能](#功能-1)
  - [运行前端](#运行前端)
- [社交平台](#社交平台)
  - [功能](#功能-2)
  - [GitHub 个人资料嵌入小组件](#github-个人资料嵌入小组件)
  - [GitHub 个人资料徽章](#github-个人资料徽章)
  - [入门](#入门)
  - [数据验证](#数据验证)
- [Wrapped 2025](#wrapped-2025)
  - [命令](#命令)
  - [包含内容](#包含内容)
- [开发](#开发)
  - [先决条件](#先决条件-1)
  - [运行方法](#运行方法)
- [支持的平台](#支持的平台)
  - [原生模块目标](#原生模块目标)
  - [Windows 支持](#windows-支持)
- [会话数据保留](#会话数据保留)
- [数据源](#数据源)
- [定价](#定价)
- [贡献](#贡献)
  - [开发指南](#开发指南)
- [致谢](#致谢)
- [许可证](#许可证)

## 功能

- **交互式 TUI 模式** - 由 Ratatui 驱动的精美终端 UI（默认模式）
  - 6 个交互式视图：概览、模型、每日、每时、统计、代理（可选的 Minutely 视图通过 `minutelyTabEnabled` 启用）
  - 键盘和鼠标导航
  - 支持可配置颜色主题的 GitHub 风格贡献图
  - 实时筛选和排序
  - 零闪烁渲染
- **多平台支持** - 跟踪 OpenCode、Claude Code、Codex CLI、Copilot CLI、Cursor IDE、Gemini CLI、Amp、Codebuff、Droid、OpenClaw、Hermes Agent、Pi、Kimi CLI、Qwen CLI、Roo Code、Kilo、Mux、Kilo CLI、Crush、Goose、Antigravity、Antigravity CLI、Zed、Kiro、Trae、Warp/Oz、Cline、Gajae-Code、Grok Build、Jcode、MiMo Code、Command Code、Junie、ZCode、OpenCodeReview、CodeBuddy、WorkBuddy、Devin CLI、Devin Desktop 和 Synthetic 的使用情况
- **实时定价** - 从 LiteLLM 获取当前价格，带 1 小时磁盘缓存；OpenRouter 自动回退和新模型的 Cursor 定价支持
- **详细分解** - 输入、输出、缓存读写和推理 Token 跟踪
- **原生 Rust 核心** - 所有解析和聚合在 Rust 中完成，处理速度提升 10 倍
- **Web 可视化** - 带 2D 和 3D 视图的交互式贡献图
- **灵活筛选** - 按平台、日期范围或年份筛选
- **任务归因报告** - 由 LLM 驱动的会话摘要与任务分组，支持多种后端（Apple FM、Claude、Codex、Gemini、Kiro）
- **导出为 JSON** - 为外部可视化工具生成数据
- **社交平台** - 分享使用情况、排行榜竞争、查看公开个人资料

## 安装

### 快速开始

```bash
# 直接用 npx 运行
npx tokscale@latest

# 或使用 bunx
bunx tokscale@latest

# 或使用 Deno（无需安装别名）
deno x npm:tokscale@latest

# 轻量模式（仅表格渲染）
npx tokscale@latest --light
```

就这样！零配置即可获得完整的交互式 TUI 体验。

> **包结构**：`tokscale` 是一个别名包（类似 [`swc`](https://www.npmjs.com/package/swc)），它安装 `@tokscale/cli`。两者都安装包含原生 Rust 核心（`@tokscale/core`）的相同 CLI。


### 先决条件

- [Node.js](https://nodejs.org/) 或 [Bun](https://bun.sh/)
- （可选）从源码构建原生模块的 Rust 工具链

### 开发环境设置

本地开发或从源码构建：

```bash
# 克隆仓库
git clone https://github.com/junhoyeo/tokscale.git
cd tokscale

# 安装 Bun（如果尚未安装）
curl -fsSL https://bun.sh/install | bash

# 安装依赖
bun install

# 开发模式运行 CLI
bun run cli
```

> **注意**：`bun run cli` 用于本地开发。通过 `bunx tokscale` 安装后，命令直接运行。下面的使用部分显示已安装的二进制命令。

### 构建原生模块

原生 Rust 模块是 CLI 操作**必需**的。它通过并行文件扫描和 SIMD JSON 解析提供约 10 倍的处理速度：

```bash
# 构建原生核心（从仓库根目录运行）
bun run build:core
```

> **注意**：通过 `bunx tokscale@latest` 安装时，原生二进制文件已预构建并包含在内。仅在本地开发时才需要从源码构建。

## 使用方法

### 基本命令

```bash
# 启动交互式 TUI（默认）
tokscale

# 使用特定标签启动 TUI
tokscale models    # 模型标签
tokscale monthly   # 每日视图（显示每日分解）

# 使用传统 CLI 表格输出
tokscale --light
tokscale models --light

# 明确启动 TUI
tokscale tui

# 导出贡献图数据为 JSON
tokscale graph --output data.json

# 以 JSON 输出数据（用于脚本/自动化）
tokscale --json                    # 默认模型视图为 JSON
tokscale models --json             # 模型分解为 JSON
tokscale monthly --json            # 月度分解为 JSON
tokscale models --json > report.json   # 保存到文件
```

### TUI 功能

交互式 TUI 模式提供：

- **8 个视图**：概览（图表 + 热门模型）、Usage（订阅配额）、模型、每日、每时、统计（贡献图）、代理。按分钟视图（Minutely）默认隐藏，可在 `settings.json` 中通过 `minutelyTabEnabled` 启用 —— 参见[配置](#配置)
- **键盘导航**：
  - `←/→/Tab/BackTab`：切换视图
  - `↑/↓` 或 `Home/End`：导航列表
  - `Enter`：打开每日详情（Daily 标签）/ 选择图表单元格（Stats 标签）
  - `Esc` 或 `Backspace`：关闭对话框 / 退出详情视图
  - `c/d/t`：按成本/日期/Token 排序
  - `j`：跳转到今天
  - `s`：打开来源选择对话框
  - `g`：打开分组方式选择对话框（模型、客户端+模型、客户端+提供商+模型、工作区+模型、会话+模型、客户端+会话+模型）
  - `h`：切换日/时图表粒度（Overview 标签）
  - `v`：切换表格/Profile 视图（Hourly 标签）
  - `y`：复制选中行到剪贴板
  - `p`：循环颜色主题
  - `r`：刷新数据；`Shift+R` 切换自动刷新；`+`/`-` 调整间隔
  - `e`：导出为 JSON
  - `q` 或 `Ctrl+C`：退出
- **鼠标支持**：点击标签、按钮和筛选器
- **主题**：Green、Halloween、Teal、Blue、Pink、Purple、Orange、Monochrome、YlGnBu、Graphite、Lagoon、Dusk
- **设置持久化**：偏好设置保存到 `~/.config/tokscale/settings.json`（参见[配置](#配置)）

### 分组策略

在 TUI 中按 `g` 或在 `--light`/`--json` 模式下使用 `--group-by` 来控制模型行的聚合方式：

| 策略 | 标志 | TUI 默认 | 效果 |
|------|------|---------|------|
| **模型** | `--group-by model` | ✅ | 每个模型一行 — 合并所有客户端和提供商 |
| **客户端 + 模型** | `--group-by client,model` | | 每个客户端-模型对一行 |
| **客户端 + 提供商 + 模型** | `--group-by client,provider,model` | | 最详细 — 不合并 |
| **工作区 + 模型** | `--group-by workspace,model` | | 先按工作区键、再按模型对本地使用量分组 |
| **会话 + 模型** | `--group-by session,model` | | 每个 `session_id` 和模型一行 — 将成本归因到特定的 agent-CLI 会话 |
| **客户端 + 会话 + 模型** | `--group-by client,session,model` | | 每个客户端、会话和模型一行 — 适用于按 `session_id` 关联的多代理运行器 |

**`--group-by model`**（最精简）

| 客户端 | 提供商 | 模型 | 费用 |
|--------|--------|------|------|
| OpenCode, Claude, Amp | github-copilot, anthropic | claude-opus-4-5 | $2,424 |
| OpenCode, Claude | anthropic, github-copilot | claude-sonnet-4-5 | $1,332 |

**`--group-by client,model`**（CLI 默认）

| 客户端 | 提供商 | 模型 | 费用 |
|--------|--------|------|------|
| OpenCode | github-copilot, anthropic | claude-opus-4-5 | $1,368 |
| Claude | anthropic | claude-opus-4-5 | $970 |

**`--group-by client,provider,model`**（最详细）

| 客户端 | 提供商 | 模型 | 费用 |
|--------|--------|------|------|
| OpenCode | github-copilot | claude-opus-4-5 | $1,200 |
| OpenCode | anthropic | claude-opus-4-5 | $168 |
| Claude | anthropic | claude-opus-4-5 | $970 |

**`--group-by session,model`**（按会话归因成本）

`tokscale models --json --group-by session,model` 会为每个 `(session_id, model)` 输出一个条目。每个条目都包含顶层的 `sessionId` 字段，以便下游工具（例如多代理 IDE）能将成本数据关联回特定的 agent-CLI 会话：

```json
{
  "groupBy": "session,model",
  "entries": [
    {
      "sessionId": "019e1e27-af49-7cd1-89b7-7bad1c3f3be2",
      "client": "codex",
      "provider": "openai",
      "model": "gpt-5",
      "input": 25251,
      "output": 47,
      "cacheRead": 1920,
      "cacheWrite": 0,
      "reasoning": 40,
      "messageCount": 12,
      "cost": 0.0123
    }
  ]
}
```

当你还需要每行都带有客户端名称时，请使用 `--group-by client,session,model`（一次涵盖全部 20+ 个受支持的 CLI）。

### 按平台筛选

使用 `--client`（短选项 `-c`）将报告范围限定为一个或多个客户端。该选项可重复使用，支持逗号分隔的值，并适用于所有报告命令：

```bash
# 仅显示 OpenCode 使用量
tokscale --client opencode

# 逗号分隔：同时筛选多个客户端
tokscale --client opencode,claude

# 重复使用：效果相同（与 shell 别名搭配使用很方便）
tokscale -c opencode -c claude

# Cursor IDE 需要先运行 `tokscale cursor login`
tokscale --client cursor

# Synthetic（synthetic.new）从其他 agent 会话中检测
tokscale --client synthetic

# 与其他筛选条件组合
tokscale --client opencode,claude --week --json
```

可用值：`opencode`、`claude`、`codex`、`copilot`、`gemini`、`cursor`、`amp`、`codebuff`、`droid`、`openclaw`、`hermes`、`pi`、`kimi`、`qwen`、`roocode`、`kilocode`、`kilo`、`mux`、`crush`、`goose`、`antigravity`、`antigravity-cli`、`zed`、`kiro`、`trae`、`warp`、`cline`、`gjc`、`grok`、`jcode`、`micode`、`commandcode`、`junie`、`zcode`、`synthetic`。

> **破坏性变更（v4.0.0）**：单客户端布尔选项（`--opencode`、`--claude`、`--codex` 等）已被移除，现在会直接报错。请改用规范的 `--client`/`-c` 选项——例如 `tokscale --client opencode,claude`。

### 日期筛选

日期筛选器适用于所有生成报告的命令（`tokscale`、`tokscale models`、`tokscale monthly`、`tokscale graph`）：

```bash
# 快速日期快捷方式
tokscale --today              # 仅今天
tokscale --yesterday          # 仅昨天
tokscale --week               # 最近 7 天
tokscale --month              # 本月

# 自定义日期范围（包含，本地时区）
tokscale --since 2024-01-01 --until 2024-12-31

# 按年份筛选
tokscale --year 2024

# 与其他选项组合
tokscale models --week --client claude --json
tokscale monthly --month --benchmark
```

> **注意**：日期筛选器使用本地时区。`--since` 和 `--until` 都是包含的。
> **v2.2.0 说明**：会话活跃时间的每日分桶也使用本地时区；非 UTC 用户会看到活跃时间日期与本地 token/费用报告日期对齐，而不是 UTC 日边界。

### 价格查询

查询任何模型的实时价格：

```bash
# 查询模型价格
tokscale pricing "claude-3-5-sonnet-20241022"
tokscale pricing "gpt-4o"
tokscale pricing "grok-code"

# 强制指定提供商来源
tokscale pricing "grok-code" --provider openrouter
tokscale pricing "claude-3-5-sonnet" --provider litellm

# 查看自定义价格覆盖
tokscale pricing list-overrides
```

**查询策略：**

价格查询使用多步解析策略：

1. **自定义价格覆盖** - 来自 `~/.config/tokscale/custom-pricing.json` 的用户自定义精确条目
2. **精确匹配** - 在 LiteLLM/OpenRouter 数据库中直接查找
3. **别名解析** - 解析友好名称（例如：`big-pickle` → `glm-4.7`）
4. **层级后缀剥离** - 移除质量层级（`gpt-5.2-xhigh` → `gpt-5.2`）
5. **版本标准化** - 处理版本格式（`claude-3-5-sonnet` ↔ `claude-3.5-sonnet`）
6. **提供商前缀匹配** - 尝试常见前缀（`anthropic/`、`openai/` 等）
7. **Cursor 模型定价** - LiteLLM/OpenRouter 中尚未收录的模型的硬编码定价（例如：`gpt-5.3-codex`）
8. **模糊匹配** - 部分模型名称的词边界匹配

### 自定义价格覆盖

在 Tokscale 的配置目录中创建 `custom-pricing.json`（macOS/Linux 上默认为 `~/.config/tokscale/custom-pricing.json`；若设置了 `TOKSCALE_CONFIG_DIR`，则为其解析出的同一目录），以覆盖上游价格数据库尚未正确收录的模型 ID 的价格。

```json
{
  "$schema": "https://tokscale.ai/custom-pricing.schema.json",
  "models": {
    "accounts/fireworks/routers/kimi-k2p6-turbo": {
      "input_cost_per_million_tokens": 2.00,
      "output_cost_per_million_tokens": 8.00,
      "cache_read_input_token_cost_per_million_tokens": 0.30,
      "source": "https://docs.fireworks.ai/serverless/pricing",
      "notes": "Fireworks Kimi K2.6 Turbo (preview)"
    },
    "accounts/fireworks/models/kimi-k2p6": {
      "input_cost_per_million_tokens": 0.95,
      "output_cost_per_million_tokens": 4.00,
      "cache_read_input_token_cost_per_million_tokens": 0.16
    },
    "kimi-k2p6-turbo": {
      "input_cost_per_million_tokens": 2.00,
      "output_cost_per_million_tokens": 8.00
    }
  }
}
```

覆盖价格以每百万 Token 的美元数输入，这与大多数 API 提供商公布价格的方式一致；Tokscale 会在内部将其转换为每 Token 的费率。`input_cost_per_million_tokens` 或 `output_cost_per_million_tokens` 中至少要有一个存在且为正值，缓存读取/缓存创建字段为可选。为兼容复制粘贴，也接受 LiteLLM 风格的每 Token 字段名，例如 `input_cost_per_token`、`output_cost_per_token` 和 `cache_read_input_token_cost`，但推荐面向用户使用每百万的命名形式。要省略某个层级或缓存价格，直接不写该字段即可；负值或非有限值会被视为无效，并跳过整个模型条目，以免拼写错误悄悄改变统计。可选的 `source` 和 `notes` 字段会被 Tokscale 忽略，可用于您自己的记账。

覆盖是仅精确匹配且不区分大小写的。Tokscale 先检查原始模型 ID，再检查现有的合成 `/models/` 归一化，然后才在没有覆盖匹配时回退到 LiteLLM、OpenRouter、Cursor 定价和模糊匹配。原始精确匹配优先于归一化精确匹配，因此 `accounts/fireworks/routers/kimi-k2p6-turbo` 可以覆盖某个特定网关的模型，而 `kimi-k2p6-turbo` 可以覆盖归一化的 `/models/` 路径。覆盖在启动时仅加载一次；编辑文件后请重启命令。这是在等待上游 LiteLLM 价格更新期间，针对错误模型定价 Bug 的推荐本地修复方案。

**提供商优先级：**

当存在多个匹配时，原始模型创建者优先于经销商：

| 优先（原创） | 次优先（经销商） |
|---------------------|-------------------------|
| `xai/`（Grok） | `azure_ai/` |
| `anthropic/`（Claude） | `bedrock/` |
| `openai/`（GPT） | `vertex_ai/` |
| `google/`（Gemini） | `together_ai/` |
| `meta-llama/` | `fireworks_ai/` |

示例：`grok-code` 匹配 `xai/grok-code-fast-1`（$0.20/$1.50）而非 `azure_ai/grok-code-fast-1`（$3.50/$17.50）。

### 社交平台命令

```bash
# 登录 Tokscale（打开浏览器进行 GitHub 认证）
tokscale login

# 无需浏览器认证即可保存已有的 Tokscale API token
tokscale login --token tt_xxx

# 查看当前登录用户
tokscale whoami

# 将已保存的 API token 显示为二维码（便于分享到另一台设备）
# 编码内容为 {"token":"tt_xxx","username":"..."} —— 可用任意二维码扫描器扫描
tokscale qr

# 提交使用量数据到排行榜
tokscale submit

# 在 CI/无头环境中提交，且不写入凭据
# 优先级：TOKSCALE_API_TOKEN 环境变量 > 已保存的凭据文件（~/.config/tokscale/credentials.json）。
# 设置了该环境变量时，本次调用会忽略已保存的文件。
TOKSCALE_API_TOKEN=tt_xxx tokscale submit

# 撤销 token：访问排行榜站点的 Settings > API Tokens
#（https://tokscale.ai/settings），点击对应 token 行的 "Revoke"。
# 撤销立即生效 —— 之后使用该 token 的请求将收到 HTTP 401 "Invalid API token"。

# 带筛选提交
tokscale submit --client opencode,claude --since 2024-01-01

# 预览将要提交的内容（试运行）
tokscale submit --dry-run

# 登出
tokscale logout
```

<img alt="CLI Submit" src="./.github/assets/cli-submit.png" />

### Autosubmit

Autosubmit 通过操作系统的调度器来安排常规的 `tokscale submit` 流程。它可以让你无需手动运行终端命令即可保持公开资料的最新状态。

```bash
# 启用定期提交。macOS 使用 launchd，Linux 在可用时使用 systemd 用户定时器
# （回退到 cron），Windows 使用 Windows 任务计划程序。
tokscale autosubmit enable --interval 24h

# 可以沿用与 submit 相同的客户端/日期筛选参数。
tokscale autosubmit enable --interval 2h --client opencode,claude --week

# 显示已保存的设置以及最近一次运行/错误。
tokscale autosubmit status
tokscale autosubmit status --json

# 即使保存的间隔尚未到达，也立即运行一次。
tokscale autosubmit run --force

# 禁用 autosubmit 并移除调度器条目。
tokscale autosubmit disable
```

计划任务的运行是非交互式的：它们不会提示 GitHub 认证或点星确认。请先运行一次 `tokscale login --token tt_xxx`，或在调度器环境中设置 `TOKSCALE_API_TOKEN`。Tokscale 会将调度器状态记录在 `settings.json` 中，将日志写入 `~/.config/tokscale/autosubmit/`，并使用锁文件确保调度器的多次触发不会重复提交。

### Cursor IDE 命令

Cursor IDE 需要通过会话令牌进行单独认证（与社交平台登录不同）：

```bash
# 登录 Cursor（需要从浏览器获取会话令牌）
# --name 是可选的，用于之后区分账户的标签
tokscale cursor login --name work

# 检查 Cursor 认证状态和会话有效性
tokscale cursor status

# 列出已保存的 Cursor 账户
tokscale cursor accounts

# 切换活动账户（同步到 cursor-cache/usage.csv 的账户）
tokscale cursor switch work

# 登出指定账户（保留历史，但不再参与合并统计）
tokscale cursor logout --name work

# 登出并删除该账户的缓存
tokscale cursor logout --name work --purge-cache

# 登出所有 Cursor 账户（保留历史，但不再参与合并统计）
tokscale cursor logout --all

# 登出所有账户并删除缓存
tokscale cursor logout --all --purge-cache
```

**凭据存储**：Cursor 账户保存到 `~/.config/tokscale/cursor-credentials.json`。使用量数据缓存在 `~/.config/tokscale/cursor-cache/`（活动账户使用 `usage.csv`，其他账户使用 `usage.<account>.csv`）。

默认情况下，tokscale 会 **合并统计所有已保存 Cursor 账户的使用量**（`cursor-cache/usage*.csv`）。为保持兼容性，活动账户会同步到 `cursor-cache/usage.csv`。

登出时，tokscale 会将缓存的历史记录移动到 `cursor-cache/archive/`（因此不会参与合并统计）。如需彻底删除缓存，请使用 `--purge-cache`。

**获取 Cursor 会话令牌的方法：**
1. 在浏览器中打开 https://www.cursor.com/settings
2. 打开开发者工具（F12）
3. **选项 A - Network 标签**：在页面上执行任何操作，找到对 `cursor.com/api/*` 的请求，在 Request Headers 中查看 `Cookie` 头，仅复制 `WorkosCursorSessionToken=` 后面的值
4. **选项 B - Application 标签**：转到 Application → Cookies → `https://www.cursor.com`，找到 `WorkosCursorSessionToken` cookie，复制其值（不是 cookie 名称）

> ⚠️ **安全警告**：像对待密码一样对待您的会话令牌。切勿公开分享或提交到版本控制。该令牌授予对您 Cursor 账户的完全访问权限。

### Antigravity 命令

Antigravity 同步目前仅在 macOS 和 Linux 上受支持。只有在启用了 Antigravity 的编辑器正在运行且本地语言服务器可用时才生效；tokscale 会从该本地语言服务器读取使用量，并将归一化后的产物缓存到本地。

```bash
# 检查 tokscale 是否能识别正在运行的 Antigravity 语言服务器
tokscale antigravity status

# 将本地 Antigravity 语言服务器中的使用量同步到 tokscale 的缓存
tokscale antigravity sync

# 删除已缓存的 Antigravity 产物
tokscale antigravity purge-cache
```

**缓存位置**：`~/.config/tokscale/antigravity-cache/`

**工作原理**：`tokscale antigravity sync` 会发现本地的 Antigravity 会话候选项，从本地语言服务器 RPC 获取已确认的使用数据，并以归一化的 JSONL 产物形式存储，供 tokscale-core 后续解析。如果希望报告反映最新的 Antigravity 数据，请在生成报告前先运行同步。

### Trae 命令

Trae（[ByteDance 的 AI IDE](https://www.trae.ai/)）有两个国际版产品线 —— Trae IDE 和 Trae Solo。它们共享同一份账号级使用量数据（同后端、同 JWT），所以 tokscale 将其统一报告为一个 `trae` 客户端。装一个或两个都可以，tokscale 会自动从任何已安装的桌面端发现凭据。

凭据通过 `--variant` 按桌面端区分：

- **`--variant ide`** —— 来自 Trae IDE 的凭据（`~/Library/Application Support/Trae/`）
- **`--variant solo`** —— 来自 Trae Solo 的凭据（`~/Library/Application Support/TRAE SOLO/`）

`tokscale trae sync` 每次运行只调用一次官方的 `query_user_usage_group_by_session` API（无论安装了几个桌面端），并将原始 JSON 持久化到本地缓存。

```bash
# 登录（自动从任意已安装的 Trae 桌面客户端检测凭据）
tokscale trae login

# 手动输入 JWT（适用于无法自动找到 storage.json 的环境，
# 例如 Linux/Windows 或无头服务器）。在浏览器打开
# https://www.trae.ai/account-setting#usage，F12 → Network →
# 过滤 `query_user_usage` → 复制 `Authorization` header 的值。
tokscale trae login --manual --variant solo

# 查看哪些变体已有缓存凭据
tokscale trae status

# 同步使用量（使用第一个可用的凭据来源）
tokscale trae sync --since 30

# 清除某个变体的缓存凭据
tokscale trae logout --variant solo
```

**缓存位置**：`~/.config/tokscale/trae-cache/`

**工作原理**：tokscale 会解密桌面客户端的 `iCubeAuthInfo://*` blob（`globalStorage/storage.json`）来恢复 JWT，或接受通过 `--manual` 粘贴的 JWT。随后它分页调用 `POST /trae/api/v1/pay/query_user_usage_group_by_session` 并保存原始 JSON。如果希望报告反映最新的 Trae 数据，请在生成报告前先运行同步。

> **中国区版本**：中国区版本（`trae.com.cn`）目前有意不支持。CN 后端暂未暴露按会话查询使用量的官方 API；如果上游提供正式端点，再加入支持。

### Warp/Oz 命令

Warp/Oz 不提供本地 token 转录。Tokscale 仅同步 Warp GraphQL API 返回的汇总请求数和消费计数器，并将其以 `warp` / `aggregate-requests` 行（token 字段均为零）的形式上报。

```bash
# 保存从已认证的 Warp 请求中复制的 bearer token 或 Cookie 头
tokscale warp login

# 检查凭据/缓存状态和诊断信息
tokscale warp status

# 将汇总请求数和消费同步到 tokscale 本地缓存
tokscale warp sync

# 移除已保存的凭据；添加 --purge-cache 可同时删除已同步的使用数据
tokscale warp logout --purge-cache
```

**缓存位置**：`~/.config/tokscale/warp-cache/usage.json`

**工作原理**：`tokscale warp sync` 调用 Warp 已认证的 GraphQL API，获取账号和工作区的汇总计数器。Tokscale 将请求数保留为消息计数，将供应商上报的消费金额保留为成本，但不会将请求数转换为合成 token。由于公开排行榜只接受基于 token 归因的使用量，Warp 数据默认不包含在 `submit` 提交内容中。

### 任务归因报告

`report` 命令会生成按任务归因的使用量分解。它使用 LLM 将每个会话总结为一个简短的标题和类别，然后将相关会话归并为高层级的任务集群，从而鸟瞰你的 Token 都花在了哪里。

```bash
# 基本报告（今天，默认使用 Apple FM 摘要器）。LLM 摘要默认开启。
tokscale report

# 最近 7 天
tokscale report --week

# 使用 Claude Code 作为摘要器后端
tokscale report --week --summarizer claude

# 使用 Codex、Gemini 或 Kiro
tokscale report --summarizer codex
tokscale report --summarizer gemini
tokscale report --summarizer kiro

# 跳过 LLM 摘要（仅显示原始数据）；这是退出（opt-out）选项
tokscale report --no-summarize

# 从头重新摘要（重置范围内已缓存的摘要）
tokscale report --week --rebuild

# 以 JSON 输出
tokscale report --week --json

# 按工作区或客户端筛选
tokscale report --workspace my-project --client opencode
```

**摘要器后端：**

| 后端 | 命令 | 说明 |
|---------|---------|-------|
| `apple-fm` | （默认） | 通过原生 Rust FFI 在本地使用 Apple Foundation Models（无需 Python）。已在预构建的 Apple Silicon（macOS arm64）二进制中启用，在开启 Apple Intelligence 的 macOS 26 及以上系统运行；在其他环境（Intel Mac、更旧的 macOS、Linux、Windows）则透明回退至内置 Rust 启发式分类器，因此默认配置可在所有平台正常使用。 |
| `claude` | `claude -p` | 需要已安装并已认证的 Claude Code CLI。 |
| `codex` | `codex --quiet` | 需要已安装并已认证的 Codex CLI。 |
| `gemini` | `gemini -p` | 需要已安装并已认证的 Gemini CLI。 |
| `kiro` | `kiro --non-interactive` | 需要已安装并已认证的 Kiro CLI。 |

**工作原理：**

1. 扫描会话并将其插入本地 SQLite wiki 数据库（`wiki.db`，位于平台配置目录——Linux 上为 `~/.config/tokscale/`，macOS 上为 `~/Library/Application Support/tokscale/`）
2. 未摘要的会话分批发送到选定的 LLM 后端，后端为每个会话返回标题、类别、描述和复杂度
3. 第二轮 LLM 处理将所有已加标题的会话归并为 3–8 个高层级任务集群（例如 "Kiro Auth"、"Tokscale Report"、"System Config"）
4. 结果缓存在 wiki 数据库中 —— 后续运行会跳过已摘要的会话

**示例输出：**

```
  Task Group                                  Sess     Tokens     Cost
  ───────────────────────────────────────────────────────────────────────
  Tokscale Development                          19      4.2B    $22.66
    Add task-attributed report command
    Implement wiki DB schema
    Fix pricing lookup for new models
  System Config                                 28      2.1B    $10.06
    Configure OpenCode workspace settings
    Update shell aliases
  Kiro Auth                                      4    890.5M     $3.10
    Implement JWT refresh flow
```

### 订阅使用量

Tokscale 可以获取并显示您在各 AI 提供商上的实时订阅配额。它会显示您已使用了多少套餐额度，以及限额何时重置。

```bash
# 显示所有已检测到提供商的订阅使用量
tokscale usage

# 以 JSON 输出（用于脚本）
tokscale usage --json

# 轻量终端输出（无 TUI）
tokscale usage --light
```

在 TUI 中，切换到 **Usage** 标签即可查看订阅数据。使用 `[Refresh]` 刷新订阅配额。键盘刷新快捷键 `r` 使用相同的刷新路径。

> **注意**：订阅配额和余额均为**供应商上报**——tokscale 调用每个提供商自己的配额端点，并原样呈现其返回结果。这些数字反映的是提供商上报的值（也就是其官方仪表板上显示的值），并未与 tokscale 自身的使用量跟踪进行独立核对。

#### 支持的提供商

| 提供商 | 认证方式 | 指标 | 设置 |
|----------|-------------|---------|-------|
| **Claude** | OAuth（凭据文件或 macOS 钥匙串） | Session（5 小时）、Weekly、Opus 配额 | 运行 `claude` 登录 |
| **Codex**（OpenAI） | OAuth（`~/.config/codex/auth.json`、`~/.codex/auth.json`，或已保存的 Tokscale 账号） | Session、Weekly 配额 | 在 TUI Usage 标签中使用 `[Add Codex]`，运行 `codex` 登录，或用 `tokscale codex import --name work` 导入现有认证 |
| **Z.ai** | API key（环境变量） | Token 限额、Web Searches | 设置 `ZAI_API_KEY` 或 `GLM_API_KEY` |
| **Amp** | API key（`~/.local/share/amp/secrets.json`） | 免费额度余额、Credits | 运行 `amp` 登录 |
| **GitHub Copilot** | GitHub token（钥匙串或 `~/.config/gh/hosts.yml`） | Premium interactions、Chat 配额 | 运行 `gh auth login` |
| **Grok Build** | OAuth（`~/.grok/auth.json`） | Credits、订阅套餐 | 运行 `grok login` |
| **Kimi** | OAuth（`~/.kimi/credentials/kimi-code.json`） | Session、Weekly 配额 | 运行 `kimi` 登录 |
| **MiniMax** | API key（环境变量） | 各模型的 Prompt 配额 | 设置 `MINIMAX_API_KEY` 或 `MINIMAX_API_TOKEN` |
| **MiniMax Token Plan** | API key（环境变量） | 区间 + 每周剩余百分比配额（按区域：CN minimaxi.com + Global minimax.io） | 设置 `MINIMAX_TOKEN_PLAN_CN_KEY` 和/或 `MINIMAX_TOKEN_PLAN_GLOBAL_KEY` |
| **Sakana**（Fugu） | 会话 cookie（环境变量或文件）—— 计费控制台 HTML 抓取，无公开 API | 5 小时、Weekly 配额窗口（套餐等级 + 月度价格作为元数据） | 设置 `SAKANA_SESSION_COOKIE`（参见 [docs/providers/sakana.md](docs/providers/sakana.md)） |

提供商会被自动检测——仅显示具有有效凭据的提供商。如果缺少某个提供商，请确认您已登录或设置了所需的环境变量。

#### Codex 多账号使用量

Tokscale 可以保存多个 Codex OAuth 账号用于订阅使用量显示。TUI Usage 标签会将已保存的账号归并到一个 **Codex** 区块下。活动账号以 `*` 标记；非活动账号可通过 `[Use]` 选中；移除账号使用 `[Remove]` 后接 `[Confirm]`。

要在不离开 TUI 的情况下添加账号，请在 Usage 标签中点击 `[Add Codex]`。Tokscale 会用一个临时的 `CODEX_HOME` 启动 `codex login`，在 Usage 标签中显示登录输出，将生成的认证导入 Tokscale 的账号存储，然后刷新使用量。这样可保持登录隔离，且不会切换当前的 Codex 认证；当您希望 Tokscale 将某个已保存账号写入真正的 Codex 认证文件时，点击该账号上的 `[Use]`。

CLI 命令仍然可用于脚本化或手动的账号管理：

```bash
# 将当前 Codex 认证保存为命名的 Tokscale 账号
tokscale codex import --name work

# 列出已保存的 Codex 账号
tokscale codex accounts
tokscale codex accounts --json

# 切换活动 Codex 账号并写入 Codex auth.json
tokscale codex switch work

# 停止跟踪某个已保存的 Codex 账号（仅从 Tokscale 的存储中移除——
# codex CLI 自身的 auth.json/登录状态永远不会被改动）
tokscale codex remove personal

# 检查活动账号或指定账号的订阅使用量
tokscale codex status
tokscale codex status --name personal --json
```

当存在已保存的 Codex 账号时，`tokscale usage --json` 会为每个 Codex 条目包含结构化的账号元数据，TUI 会将这些条目显示在一个 Codex 分组下。若无已保存的账号，Tokscale 会回退到当前的 Codex 认证发现路径（`CODEX_HOME/auth.json`、`~/.config/codex/auth.json`、`~/.codex/auth.json`，然后是 macOS 钥匙串）。

#### 示例输出

```
╭──────────────────────────────────────────────────────────╮
│ Session    85% left  [=========---] resets in 2h 15m     │
│ Weekly     72% left  [========----] resets Fri 3pm       │
│ Plan     Max 20x                                         │
╰──────────────────────────────────────────────────────────╯
╭──────────────────────────────────────────────────────────╮
│ Session    40% left  [=====-------] resets in 4h 30m     │
│ Weekly     90% left  [==========--] resets Mon 12am      │
│ Account  user@example.com                                │
│ Plan     Pro                                             │
╰──────────────────────────────────────────────────────────╯
```

### 示例输出（`--light` 版本）

<img alt="CLI Light" src="./.github/assets/cli-light.png" />

### 配置

Tokscale 将设置存储在 `~/.config/tokscale/settings.json`：

```json
{
  "colorPalette": "blue",
  "includeUnusedModels": false,
  "defaultClients": ["opencode", "claude"],
  "scanner": {
    "extraScanPaths": {
      "codex": [
        "/Users/me/workspace/project-a/.codex/sessions",
        "/Users/me/workspace/project-b/.codex/archived_sessions"
      ],
      "hermes": [
        "/Users/me/.hermes/profiles/director_planning",
        "/Users/me/.hermes/profiles/research/state.db"
      ]
    }
  }
}
```

| 设置 | 类型 | 默认值 | 描述 |
|---------|------|---------|-------------|
| `colorPalette` | string | `"blue"` | TUI 颜色主题（green、halloween、teal、blue、pink、purple、orange、monochrome、ylgnbu、graphite、lagoon、dusk） |
| `includeUnusedModels` | boolean | `false` | 在报告中显示零 Token 的模型 |
| `autoRefreshEnabled` | boolean | `false` | 在 TUI 中启用自动刷新 |
| `autoRefreshMs` | number | `60000` | 自动刷新间隔（30000-3600000ms） |
| `nativeTimeoutMs` | number | `300000` | 原生子进程处理最大时间（5000-3600000ms） |
| `defaultClients` | string[] | `[]` | 未传递 `--client/-c` 选项时应用的客户端筛选。接受与 `--client` 相同的 ID（例如 `["opencode", "claude", "synthetic"]`）。未知 ID 会被静默丢弃。命令行选项会完全覆盖此列表 — 不会合并。 |
| `light.writeCache` | boolean | `false` | 为 `true` 时，`tokscale --light` 会在渲染完成后以原子方式覆盖 TUI 缓存。CLI 标志 `--write-cache` / `--no-write-cache` 会按次运行覆盖该设置。 |
| `minutelyTabEnabled` | boolean | `false` | 在 TUI 中显示按分钟的 Minutely 标签，并在数据加载期间执行分钟级聚合。对大多数用户而言，分钟级粒度是较为小众的诊断视图，而在大数据集上分钟分桶有非平凡的代价，因此默认关闭。 |
| `scanner.extraScanPaths` | object | `{}` | 针对 Tokscale 默认 home 根位置之外的会话，为各客户端额外指定的扫描根目录 |

使用 `scanner.extraScanPaths` 配置持久化的额外根目录，例如项目级的 `.codex` 目录或导入的 Gemini/OpenClaw 历史。Tokscale 会自动发现 `$HERMES_HOME/profiles/*/state.db` 下的 Hermes 配置文件数据库（未设置 `HERMES_HOME` 时为 `~/.hermes/profiles/*/state.db`）。仅对非标准的 Hermes 配置文件位置使用 `scanner.extraScanPaths.hermes`；Hermes 条目既可以指向包含 `state.db` 的配置文件目录，也可以直接指向 `state.db` 文件。Tokscale 在每次运行时都会将这些路径与默认扫描根目录合并，并按规范路径去重重叠的根目录。

使用 `defaultClients` 固定一个个人默认值 —— 例如，如果您只使用 OpenCode 和 Claude，就将其设为 `["opencode", "claude"]`，那么 `tokscale`（不带任何选项）会自动将每个报告的范围限定为它们。在命令行传入 `--client` 可针对单次运行进行覆盖。

#### 启用 Minutely 标签

Minutely 标签按分钟显示 Token 使用情况，最适合用于诊断突发模式、调试单个会话，或与 `autoRefreshEnabled` 配合进行近实时监控。分钟级聚合在数据加载期间会遍历所有已解析的消息，对大多数用户来说带来不必要的 RAM 与 CPU 开销。因此默认情况下它是隐藏的。

要启用它，请在 `~/.config/tokscale/settings.json` 中将 `minutelyTabEnabled` 设为 `true`：

```json
{
  "minutelyTabEnabled": true
}
```

重启后，Minutely 标签将出现在标签栏中 Hourly 与 Stats 之间，Tab / BackTab / Left / Right 导航将会在其间循环。把该标志再设为 `false` 可隐藏标签并再次跳过聚合。

#### 缓存目录布局

可再生成的 CLI/TUI/价格/Wrapped 缓存位于 `~/.config/tokscale/cache/` 下（如果设置了 `TOKSCALE_CONFIG_DIR`，则为 `${TOKSCALE_CONFIG_DIR}/cache/`）。集成同步产物保留在各自的客户端缓存目录中，例如 `~/.config/tokscale/antigravity-cache/` 和 `~/.config/tokscale/trae-cache/`：

- `tui-data-cache.json` —— TUI 启动缓存
- `source-message-cache-v2/` + `source-message-cache.lock` —— 分片源消息缓存与锁文件
- `pricing-litellm.json` / `pricing-openrouter.json` —— 定价缓存
- `opencode-migration.json` —— OpenCode 迁移记录
- `fonts/`、`images/` —— Wrapped 资源缓存

删除该目录是安全的。Tokscale 会在需要时重新创建并重新生成其中的内容。

### 环境变量

环境变量会覆盖配置文件中的值。适用于 CI/CD 或一次性使用：

| 变量 | 默认值 | 描述 |
|----------|---------|-------------|
| `TOKSCALE_NATIVE_TIMEOUT_MS` | `300000`（5 分钟） | 覆盖 `nativeTimeoutMs` 配置 |
| `TOKSCALE_API_TOKEN` | unset | 用于非交互式 `submit` 和 `delete-submitted-data` 运行的 Tokscale 个人 API token。可从 Settings > API Tokens 创建一个，或用 `tokscale login --token tt_xxx` 保存到本地。 |
| `TOKSCALE_EXTRA_DIRS` | unset | 一次性的额外会话根目录，格式为 `client:/abs/path,client:/abs/path` |
| `TOKSCALE_CONFIG_DIR` | unset | 覆盖配置目录根（`settings.json`、`star-cache.json`、`cache/`、`antigravity-cache/`、`trae-cache/` 的存放位置）。建议使用绝对路径；相对路径将基于进程 CWD 解析。适用于 CI 沙箱或固定到非默认位置。设置后，tokscale 不会回退到 macOS 旧路径（`~/Library/Application Support/tokscale/`）。 |
| `TOKSCALE_FM_DEBUG` | unset | 设置后，会将 Apple Foundation Models 的诊断信息（macOS 版本门槛、dlopen dylib 路径、加载/符号错误）打印到 stderr，以说明本机端 apple-fm 为何启用或未启用。 |

```bash
# 示例：为非常大的数据集增加超时时间
TOKSCALE_NATIVE_TIMEOUT_MS=600000 tokscale graph --output data.json

# 示例：一次性的额外扫描根目录
TOKSCALE_EXTRA_DIRS='codex:/Users/me/workspace/project-a/.codex/sessions,gemini:/Users/me/imports/imac/gemini/tmp' tokscale

# 示例：在 CI 中提交，无需交互式浏览器登录
TOKSCALE_API_TOKEN=tt_xxx tokscale submit
```

> **注意**：对于持久化的额外根目录，建议在 `~/.config/tokscale/settings.json` 中使用 `scanner.extraScanPaths`。`TOKSCALE_EXTRA_DIRS` 最适合一次性覆盖或 CI/CD。

### Headless 模式

Tokscale 可以聚合来自 **Codex CLI 无头输出**的令牌使用情况，用于自动化、CI/CD 流水线和批处理。

**什么是 Headless 模式？**

当您使用 JSON 输出标志运行 Codex CLI 时（例如 \`codex exec --json\`），它会将使用数据输出到 stdout，而不是存储在常规会话目录中。Headless 模式允许您捕获和跟踪这些使用情况。

**存储位置：** \`~/.config/tokscale/headless/\`

在 macOS 上，当未设置 \`TOKSCALE_HEADLESS_DIR\` 时，Tokscale 也会扫描 \`~/Library/Application Support/tokscale/headless/\`。

Tokscale 会自动扫描此目录结构：
```
~/.config/tokscale/headless/
└── codex/       # Codex CLI JSONL 输出
```

**环境变量：** 设置 \`TOKSCALE_HEADLESS_DIR\` 以自定义无头日志目录：
```bash
export TOKSCALE_HEADLESS_DIR="$HOME/my-custom-logs"
```

**推荐（自动捕获）：**

| 工具 | 命令示例 |
|------|----------|
| **Codex CLI** | \`tokscale headless codex exec -m gpt-5 "implement feature"\` |

**手动重定向（可选）：**

| 工具 | 命令示例 |
|------|----------|
| **Codex CLI** | \`codex exec --json "implement feature" > ~/.config/tokscale/headless/codex/ci-run.jsonl\` |

**诊断：**

```bash
# 显示扫描位置和无头计数
tokscale sources
tokscale sources --json
```

**CI/CD 集成示例：**

```bash
# 在 GitHub Actions 工作流中
- name: Run AI automation
  run: |
    mkdir -p ~/.config/tokscale/headless/codex
    codex exec --json "review code changes" \\
      > ~/.config/tokscale/headless/codex/pr-\${{ github.event.pull_request.number }}.jsonl

# 稍后跟踪使用情况
- name: Report token usage
  run: tokscale --json
```

> **注意**：无头捕获仅支持 Codex CLI。如果直接运行 Codex，必须如上所示将 stdout 重定向到 headless 目录。

## 前端可视化

前端提供 GitHub 风格的贡献图可视化：

### 功能

- **2D 视图**：经典 GitHub 贡献日历
- **3D 视图**：基于 Token 使用量高度的等距 3D 贡献图
- **多种颜色调色板**：GitHub、GitLab、Halloween、Winter 等
- **三态主题切换**：Light / Dark / System（跟随系统设置）
- **GitHub Primer 设计**：使用 GitHub 官方颜色系统
- **交互式提示**：悬停查看详细的每日分解
- **每日分解面板**：点击查看每个来源和模型的详情
- **年份筛选**：在年份之间导航
- **来源筛选**：按平台筛选（OpenCode、Claude、Codex、Copilot、Cursor、Gemini、Amp、Codebuff、Droid、OpenClaw、Hermes Agent、Pi、Kimi、Qwen、Roo Code、Kilo、Mux、Kilo CLI、Crush、Goose、Antigravity、Antigravity CLI、Zed、Kiro、Trae、Warp、Cline、Gajae-Code、Grok Build、Jcode、MiMo Code、Command Code、Junie、ZCode、Synthetic）
- **统计面板**：总成本、Token、活跃天数、连续记录
- **FOUC 防护**：在 React 水合前应用主题（无闪烁）

### 运行前端

```bash
cd packages/frontend
bun install
bun run dev
```

打开 [http://localhost:3000](http://localhost:3000) 访问社交平台。

## 社交平台

Tokscale 包含一个社交平台，您可以在其中分享使用数据并与其他开发者竞争。

### 功能

- **排行榜** - 查看所有平台上使用最多 Token 的人
- **用户资料** - 带贡献图和统计的公开资料
- **时间段筛选** - 查看所有时间、本月或本周的统计
- **GitHub 集成** - 使用 GitHub 账户登录
- **本地查看器** - 无需提交即可私密查看数据

### GitHub 个人资料嵌入小组件

您可以直接在 GitHub 个人资料 README 中嵌入 Tokscale 公开统计数据：

```md
[![Tokscale Stats](https://tokscale.ai/api/embed/<username>/svg)](https://tokscale.ai/u/<username>)
```

将 `<username>` 替换为您的 GitHub 用户名。不带任何查询参数时，渲染默认的 `classic` 卡片；追加下面的任意参数即可自定义设计。

| 参数 | 取值 | 效果 |
| --- | --- | --- |
| `template` | `classic`（默认）· `minimal` · `terminal` · `graph` · `orbit` · `vitals` · `blueprint` · `receipt` | 卡片设计 |
| `color` | `blue` · `green` · `teal` · `purple` · `pink` · `orange` · `monochrome` · `halloween` · `YlGnBu` | 强调色和贡献图配色 |
| `theme` | `dark`（默认）· `light` | 浅色或深色卡片 |
| `sort` | `tokens`（默认）· `cost` | 排名取自哪个排行榜 |
| `tokens`、`cost` | `compact` · `full` | 数字格式，可分别设置 —— `20.9B` 对比 `20,941,000,000` |
| `rank` | `plain`（默认，`#134`）· `percent`（`top 12%`）· `total`（`#134 / 1,174`） | 排行榜名次的显示方式 |
| `graph` | `1` 表示追加贡献图（默认关闭） | `classic`、`minimal`、`terminal`、`orbit`、`blueprint`、`receipt` 支持 |
| `compact` | `1` 表示紧凑布局 | 仅 `classic` |

示例：

```md
![](https://tokscale.ai/api/embed/<username>/svg?template=minimal&color=purple&graph=1)
![](https://tokscale.ai/api/embed/<username>/svg?template=orbit&color=pink&rank=percent)
![](https://tokscale.ai/api/embed/<username>/svg?template=terminal&color=green&theme=light)
![](https://tokscale.ai/api/embed/<username>/svg?template=receipt&color=YlGnBu&graph=1)
```

### GitHub 个人资料徽章

您还可以使用 shields.io 风格的更紧凑的徽章：

```md
![Tokscale Tokens](https://tokscale.ai/api/badge/<username>/svg)
```

- 将 `<username>` 替换为您的 GitHub 用户名
- 可选查询参数：
  - `metric=tokens`（默认）、`metric=cost` 或 `metric=rank`
  - `style=flat`（默认）或 `style=flat-square`
  - `sort=tokens`（默认）或 `sort=cost` 控制排名依据
  - `compact=1` 使用紧凑数字表示（例如 `1.2M`、`$3.4K`）
  - `label=<文本>` 自定义左侧标签
  - `color=<hex>` 自定义右侧背景颜色（例如 `color=ff5733`）
- 示例：
  - `https://tokscale.ai/api/badge/<username>/svg?metric=cost&compact=1`
  - `https://tokscale.ai/api/badge/<username>/svg?metric=rank&sort=cost&style=flat-square`

### 入门

1. **登录** - 运行 `tokscale login` 通过 GitHub 认证，或在 Settings 中创建一个 API token 供 CI/无头环境使用
2. **提交** - 运行 `tokscale submit` 上传使用数据
3. **查看** - 访问 Web 平台查看您的资料和排行榜

### 数据验证

提交的数据经过一级验证：
- 数学一致性（总计匹配，无负值）
- 无未来日期
- 必填字段存在
- 重复检测

## Wrapped 2025

![Wrapped 2025](.github/assets/hero-wrapped-2025.png)

生成一张精美的年度回顾图片，总结您的 AI 编程助手使用情况——灵感来自 Spotify Wrapped。

| `bunx tokscale@latest wrapped` | `bunx tokscale@latest wrapped --clients` | `bunx tokscale@latest wrapped --agents --disable-pinned` |
|:---:|:---:|:---:|
| ![Wrapped 2025 (Agents + Pin Sisyphus)](.github/assets/wrapped-2025-agents.png) | ![Wrapped 2025 (Clients)](.github/assets/wrapped-2025-clients.png) | ![Wrapped 2025 (Agents + Disable Pinned)](.github/assets/wrapped-2025-agents-disable-pinned.png) |

### 命令

```bash
# 生成当前年份的 Wrapped 图片
tokscale wrapped

# 生成指定年份的 Wrapped 图片
tokscale wrapped --year 2025
```

### 包含内容

生成的图片包括：

- **总 Token 数** - 您当年的总 Token 消耗量
- **热门模型** - 按成本排名的前 3 个最常用 AI 模型
- **热门客户端** - 前 3 个最常用平台（OpenCode、Claude Code、Cursor 等）
- **消息数** - AI 交互总数
- **活跃天数** - 至少有一次 AI 交互的天数
- **成本** - 基于 LiteLLM 定价的估计总成本
- **连续记录** - 最长连续活跃天数
- **贡献图** - 年度活动的可视化热力图

生成的 PNG 已针对社交媒体分享进行优化。与社区分享您的编程之旅！

## 开发

> **快速设置**：如果您只想快速开始，请参阅上面安装部分的[开发环境设置](#开发环境设置)。

### 先决条件

```bash
# Bun（必需）
bun --version

# Rust（用于原生模块）
rustc --version
cargo --version
```

### 运行方法

按照[开发环境设置](#开发环境设置)后，您可以：

```bash
# 构建原生模块（可选但推荐）
bun run build:core

# 以开发模式运行（启动 TUI）
cd packages/cli && bun src/index.ts

# 或使用传统 CLI 模式
cd packages/cli && bun src/index.ts --light
```

<details>
<summary>高级开发</summary>

### 项目脚本

| 脚本 | 描述 |
|--------|-------------|
| `bun run cli` | 开发模式运行 CLI（使用 Bun 的 TUI） |
| `bun run build:core` | 构建原生 Rust 模块（发布版） |
| `bun run build:cli` | 将 CLI TypeScript 构建到 dist/ |
| `bun run build` | 同时构建 core 和 CLI |
| `bun run dev:frontend` | 运行前端开发服务器 |

**特定包脚本**（从包目录内）：
- `packages/cli`：`bun run dev`、`bun run tui`
- `packages/core`：`bun run build:debug`、`bun run test`、`bun run bench`

**注意**：此项目使用 **Bun** 作为开发时的包管理器。

### 测试

```bash
# 测试原生模块（Rust）
cd packages/core
bun run test:rust      # Cargo 测试
bun run test           # Node.js 集成测试
bun run test:all       # 两者都
```

### 原生模块开发

```bash
cd packages/core

# 调试模式构建（编译更快）
bun run build:debug

# 发布模式构建（优化版）
bun run build

# 运行 Rust 基准测试
bun run bench
```

### 图表命令选项

```bash
# 导出图表数据到文件
tokscale graph --output usage-data.json

# 日期筛选（所有快捷方式都有效）
tokscale graph --today
tokscale graph --week
tokscale graph --since 2024-01-01 --until 2024-12-31
tokscale graph --year 2024

# 按平台筛选
tokscale graph --client opencode,claude

# 显示处理时间基准
tokscale graph --output data.json --benchmark
```

### 基准测试标志

显示处理时间以进行性能分析：

```bash
tokscale --benchmark           # 显示默认视图的处理时间
tokscale models --benchmark    # 基准测试模型报告
tokscale monthly --benchmark   # 基准测试月度报告
tokscale graph --benchmark     # 基准测试图表生成
```

### 为前端生成数据

```bash
# 导出可视化数据
tokscale graph --output packages/frontend/public/my-data.json
```

### 性能

原生 Rust 模块提供显著的性能提升：

| 操作 | TypeScript | Rust 原生 | 加速 |
|-----------|------------|-------------|---------|
| 文件发现 | ~500ms | ~50ms | **10 倍** |
| JSON 解析 | ~800ms | ~100ms | **8 倍** |
| 聚合 | ~200ms | ~25ms | **8 倍** |
| **总计** | **~1.5 秒** | **~175ms** | **~8.5 倍** |

*约 1000 个会话文件、100k 消息的基准测试*

#### 内存优化

原生模块还通过以下方式提供约 45% 的内存减少：

- 流式 JSON 解析（无完整文件缓冲）
- 零拷贝字符串处理
- 使用 map-reduce 的高效并行聚合

#### 运行基准测试

```bash
# 生成合成数据
cd packages/benchmarks && bun run generate

# 运行 Rust 基准测试
cd packages/core && bun run bench
```

</details>

## 支持的平台

### 原生模块目标

| 平台 | 架构 |
|----------|--------------|
| macOS | x86_64 |
| macOS | aarch64（Apple Silicon） |
| Linux | x86_64（glibc） |
| Linux | aarch64（glibc） |
| Linux | x86_64（musl） |
| Linux | aarch64（musl） |
| Windows | x86_64 |
| Windows | aarch64 |

### Windows 支持

Tokscale 完全支持 Windows。TUI 和 CLI 的工作方式与 macOS/Linux 相同。

**Windows 安装：**
```powershell
# 安装 Bun（PowerShell）
powershell -c "irm bun.sh/install.ps1 | iex"

# 运行 tokscale
bunx tokscale@latest
```

#### Windows 上的数据位置

AI 编程工具将会话数据存储在跨平台位置。大多数工具在所有平台上使用相同的相对路径：

| 工具 | Unix 路径 | Windows 路径 | 来源 |
|------|-----------|--------------|--------|
| OpenCode | `~/.local/share/opencode/` | `%USERPROFILE%\.local\share\opencode\` | 使用 [`xdg-basedir`](https://github.com/sindresorhus/xdg-basedir) 实现跨平台一致性（[源码](https://github.com/sst/opencode/blob/main/packages/opencode/src/global/index.ts)） |
| Claude Code | `~/.claude/` | `%USERPROFILE%\.claude\` | 所有平台使用相同路径 |
| OpenClaw | `~/.openclaw/` (+ 旧版: `.clawdbot`, `.moltbot`, `.moldbot`) | `%USERPROFILE%\.openclaw\` (+ 旧版路径) | 所有平台使用相同路径 |
| Codex CLI | `~/.codex/` | `%USERPROFILE%\.codex\` | 可通过 `CODEX_HOME` 环境变量配置（[源码](https://github.com/openai/codex)） |
| Copilot CLI | `~/.copilot/otel/` | `%USERPROFILE%\.copilot\otel\` | 需要 OTEL 文件导出；同时自动采集 `COPILOT_OTEL_FILE_EXPORTER_PATH` |
| Hermes Agent | `~/.hermes/` | `%USERPROFILE%\.hermes\` | 可通过 `HERMES_HOME` 环境变量配置（[源码](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/session-storage.md)） |
| Gemini CLI | `~/.gemini/` | `%USERPROFILE%\.gemini\` | 可通过 `GEMINI_CLI_HOME` 环境变量配置 |
| Amp | `~/.local/share/amp/` | `%USERPROFILE%\.local\share\amp\` | 与 OpenCode 一样使用 `xdg-basedir` |
| Cursor | API 同步 | API 同步 | 通过 API 获取数据，缓存在 `%USERPROFILE%\.config\tokscale\cursor-cache\` |
| Droid | `~/.factory/` | `%USERPROFILE%\.factory\` | 所有平台使用相同路径 |
| Pi | `~/.pi/` and `~/.omp/` | `%USERPROFILE%\.pi\` and `%USERPROFILE%\.omp\` | 所有平台使用相同路径（支持 Pi 和 [Oh My Pi](https://github.com/can1357/oh-my-pi)） |
| Kimi CLI | `~/.kimi/` | `%USERPROFILE%\.kimi\` | 所有平台使用相同路径 |
| Kimi Code | `~/.kimi-code/` | `%USERPROFILE%\.kimi-code\` | 所有平台使用相同路径 |
| Qwen CLI | `~/.qwen/` | `%USERPROFILE%\.qwen\` | 所有平台使用相同路径 |
| Roo Code | `~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks/` | `%USERPROFILE%\.config\Code\User\globalStorage\rooveterinaryinc.roo-cline\tasks\` | VS Code globalStorage 任务日志 |
| Kilo | `~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/` | `%USERPROFILE%\.config\Code\User\globalStorage\kilocode.kilo-code\tasks\` | VS Code globalStorage 任务日志 |
| Cline | Linux: `~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/`；macOS: `~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/`；server: `~/.vscode-server/data/User/globalStorage/saoudrizwan.claude-dev/tasks/` | `%APPDATA%\Code\User\globalStorage\saoudrizwan.claude-dev\tasks\` | VS Code globalStorage 任务日志 |
| Mux | `~/.mux/sessions/` | `%USERPROFILE%\.mux\sessions\` | 所有平台相同路径 |
| Codebuff | `~/.config/manicode/projects/`（+ `manicode-dev`、`manicode-staging`） | `%USERPROFILE%\.config\manicode\projects\` | 通过 `CODEBUFF_DATA_DIR` 环境变量覆盖 |
| Kilo CLI | `~/.local/share/kilo/` | `%USERPROFILE%\.local\share\kilo\` | 与 OpenCode 一样使用 `xdg-basedir` |
| Crush | `$XDG_DATA_HOME/crush/`（回退路径：`~/.local/share/crush/`） | `%USERPROFILE%\.local\share\crush\`（如果设置了 `%XDG_DATA_HOME%`，则为 `%XDG_DATA_HOME%\crush\`） | 使用带回退路径的 XDG 数据目录 |
| Goose | `~/.local/share/goose/sessions/`（+ macOS Application Support、旧版 Block 路径） | `%USERPROFILE%\.local\share\goose\sessions\` | 可通过 `GOOSE_PATH_ROOT` 环境变量配置 |
| Antigravity | `~/.config/tokscale/antigravity-cache/sessions/` | — | `tokscale antigravity sync` 目前仅支持 macOS / Linux |
| Zed Agent | `~/.local/share/zed/threads/threads.db` | `%LOCALAPPDATA%\Zed\threads\threads.db` | 仅限托管 Zed 模型的使用量；不含外部 ACP 代理 |
| Kiro | `~/.kiro/sessions/cli/` 和 `~/.local/share/kiro-cli/data.sqlite3` | `%USERPROFILE%\.kiro\sessions\cli\` 和 `%USERPROFILE%\.local\share\kiro-cli\data.sqlite3` | 解析 Kiro 会话文件，以及存在时的 Kiro CLI SQLite 数据库 |
| Trae | `~/.config/tokscale/trae-cache/sessions/` | `%APPDATA%\tokscale\trae-cache\sessions\` | 通过 `tokscale trae sync` 同步一次；凭据会从已安装的任意 Trae IDE 或 Trae Solo 桌面端自动发现 |
| Warp/Oz | `~/.config/tokscale/warp-cache/usage.json` | `%APPDATA%\tokscale\warp-cache\usage.json` | 通过 `tokscale warp sync` 同步；仅汇总请求数和消费金额，不含 token 转录 |
| Grok Build | `~/.grok/sessions/` | `%USERPROFILE%\.grok\sessions\` | 可通过 `GROK_HOME` 环境变量配置；解析 `updates.jsonl` 会话更新 |
| Jcode | `~/.jcode/sessions/` | `%USERPROFILE%\.jcode\sessions\` | 可通过 `JCODE_HOME` 环境变量配置；解析 `session_*.json` 快照以及 `session_*.journal.jsonl` sidecar |
| MiMo Code | `~/.local/share/mimocode/` | `%USERPROFILE%\.local\share\mimocode\` | 使用 XDG 数据目录；SQLite 数据库 `mimocode.db` |
| Gajae-Code | `~/.gjc/agent/sessions/` | `%USERPROFILE%\.gjc\agent\sessions\` | 可通过 `GJC_CODING_AGENT_DIR`（也可用 `GJC_CONFIG_DIR`/`PI_CONFIG_DIR`；Linux/macOS 上 `$XDG_DATA_HOME/gjc/sessions/` 亦支持）配置 |
| Junie | `~/.junie/sessions/` | `%USERPROFILE%\.junie\sessions\` | 所有平台使用相同的 home 相对路径；解析 `events.jsonl` 使用事件 |
| ZCode | `~/.zcode/cli/db/db.sqlite` 和 `~/.zcode/projects/` | `%USERPROFILE%\.zcode\cli\db\db.sqlite` 和 `%USERPROFILE%\.zcode\projects\` | 解析 v2 SQLite 模型用量和旧版 `*.jsonl` 会话记录；Z.ai 的 GLM 模型专用 ADE |
| OpenCodeReview | `~/.opencodereview/sessions/` | `%USERPROFILE%\.opencodereview\sessions\` | 解析 `*.jsonl` 会话记录；阿里巴巴的 AI 代码审查工具 |
| CodeBuddy | `~/.codebuddy/projects/` + 扩展日志 | `%USERPROFILE%\.codebuddy\projects\` + CodeBuddy / VS Code 扩展日志 | 解析 CodeBuddy CLI、IDE 和 VS Code 插件的 token 用量 |
| WorkBuddy | `~/.workbuddy/projects/` + `~/.workbuddy/workbuddy.db` | `%USERPROFILE%\.workbuddy\projects\` + `%USERPROFILE%\.workbuddy\workbuddy.db` | 解析 WorkBuddy token 用量，以聚合 SQLite 数据库作为回退 |
| Synthetic | 从其他来源重归属 | 从其他来源重归属 | 检测 `hf:` 模型前缀 + `synthetic` provider |

> **注意**：在 Windows 上，`~` 扩展为 `%USERPROFILE%`（例如 `C:\Users\用户名`）。这些工具故意使用 Unix 风格的路径（如 `.local/share`）而不是 Windows 原生路径（如 `%APPDATA%`），以实现跨平台一致性。

#### Windows 特定配置

Tokscale 将配置存储在：
- **TUI 设置**: `%APPDATA%\tokscale\settings.json`（平台默认值。可用 `TOKSCALE_CONFIG_DIR` 覆盖）
- **缓存**: `%APPDATA%\tokscale\cache\`（统一缓存根目录）
- **旧版缓存路径**: 旧版本曾使用 `%USERPROFILE%\.cache\tokscale\` 这类分散路径；在可再生数据写入新路径之前，这些旧路径可能仍然存在。
- **Cursor 凭据**: `%USERPROFILE%\.config\tokscale\cursor-credentials.json`
- **Trae 凭据和同步使用量**: `%APPDATA%\tokscale\trae-cache\`
- **Tokscale 账号凭据**: `%USERPROFILE%\.config\tokscale\credentials.json`

## 会话数据保留

默认情况下，一些 AI 编程助手会自动删除旧的会话文件。为了准确跟踪，请禁用或延长清理周期以保留使用历史。

| 平台 | 默认值 | 配置文件 | 禁用设置 | 来源 |
|----------|---------|-------------|-------------------|--------|
| Claude Code | **⚠️ 30 天** | `~/.claude/settings.json` | `"cleanupPeriodDays": 9999999999` | [文档](https://docs.anthropic.com/en/docs/claude-code/settings) |
| Gemini CLI | 禁用 | `$GEMINI_CLI_HOME/settings.json`（回退：`~/.gemini/settings.json`） | `"general.sessionRetention.enabled": false` | [文档](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/session-management.md) |
| Codex CLI | 禁用 | N/A | 无清理功能 | [#6015](https://github.com/openai/codex/issues/6015) |
| OpenCode | 禁用 | N/A | 无清理功能 | [#4980](https://github.com/sst/opencode/issues/4980) |

### Claude Code

**默认**：30 天清理周期

添加到 `~/.claude/settings.json`：
```json
{
  "cleanupPeriodDays": 9999999999
}
```

> 设置一个非常大的值（例如 `9999999999` 天 ≈ 2700 万年）实际上会禁用清理。

### Gemini CLI

**默认**：清理已禁用（会话永久保留）

如果您已启用清理并想禁用它，请在 `$GEMINI_CLI_HOME/settings.json`（回退：`~/.gemini/settings.json`）中删除或设置 `enabled: false`：
```json
{
  "general": {
    "sessionRetention": {
      "enabled": false
    }
  }
}
```

或设置非常长的保留期：
```json
{
  "general": {
    "sessionRetention": {
      "enabled": true,
      "maxAge": "9999999d"
    }
  }
}
```

### Codex CLI

**默认**：无自动清理（会话永久保留）

Codex CLI 没有内置会话清理。`~/.codex/sessions/` 中的会话无限期保留。

> **注意**：有一个关于此功能的请求：[#6015](https://github.com/openai/codex/issues/6015)

### OpenCode

**默认**：无自动清理（会话永久保留）

OpenCode 没有内置会话清理。`~/.local/share/opencode/storage/` 中的会话无限期保留。

> **注意**：参见 [#4980](https://github.com/sst/opencode/issues/4980)

---

## 数据源

### OpenCode

位置：`~/.local/share/opencode/opencode.db` (v1.2+) 或 `storage/message/{sessionId}/*.json` (旧版)

OpenCode 1.2+ 将会话存储在 SQLite 中。Tokscale 优先从 SQLite 读取，旧版本则回退到旧版 JSON 文件。

OpenCode 根据构建时的发布渠道决定数据库文件名：`latest`/`beta` 渠道使用 `opencode.db`，其他渠道使用 `opencode-<channel>.db`（例如 `opencode-stable.db`、`opencode-nightly.db`）。Tokscale 会扫描所有这些文件，因此同时使用多个渠道的用户也能获得统一的视图。

如果您用 `OPENCODE_DB` 指向 `~/.local/share/opencode` 之外的文件来启动 opencode，请将该绝对路径添加到 `~/.config/tokscale/settings.json`，以便 tokscale 每次运行都能找到它：

```json
{
  "scanner": {
    "opencodeDbPaths": [
      "/custom/location/opencode.db",
      "/another/location/opencode-stable.db"
    ]
  }
}
```

这些路径会与自动发现结果合并，按规范路径去重，不存在的条目会被静默跳过（因此过时的配置不会让扫描中断）。`opencode.db-wal`、`opencode.db-shm` 及其他 SQLite sidecar 会被拒绝。

如果您将会话保存在 Tokscale 默认 home 根位置之外，也可以为各客户端持久化额外的扫描根目录：

```json
{
  "scanner": {
    "extraScanPaths": {
      "codex": [
        "/Users/me/workspace/project-a/.codex/sessions",
        "/Users/me/workspace/project-b/.codex/archived_sessions"
      ],
      "gemini": ["/Users/me/imports/imac/gemini/tmp"],
      "hermes": [
        "/Users/me/.hermes/profiles/director_planning",
        "/Users/me/.hermes/profiles/research/state.db"
      ],
      "openclaw": ["/Users/me/imports/imac/openclaw/agents"]
    }
  }
}
```

这对于项目级的 `.codex` 目录、导入的历史，以及默认 `$HERMES_HOME/state.db` 或 `~/.hermes/state.db` 位置之外的 Hermes 配置文件数据库都很有用。Tokscale 仍会扫描其默认根目录，然后在其上合并 `scanner.extraScanPaths` 和 `TOKSCALE_EXTRA_DIRS`，并按规范路径去重。它不会自动发现您的整个工作区。

每个消息包含：
```json
{
  "id": "msg_xxx",
  "role": "assistant",
  "modelID": "claude-sonnet-4-20250514",
  "providerID": "anthropic",
  "tokens": {
    "input": 1234,
    "output": 567,
    "reasoning": 0,
    "cache": { "read": 890, "write": 123 }
  },
  "time": { "created": 1699999999999 }
}
```

### Claude Code

位置：`~/.claude/projects/{projectPath}/*.jsonl` 和 `~/.claude/transcripts/*.jsonl`

包含使用数据的助手消息的 JSONL 格式：
```json
{"type": "assistant", "message": {"model": "claude-sonnet-4-20250514", "usage": {"input_tokens": 1234, "output_tokens": 567, "cache_read_input_tokens": 890}}, "timestamp": "2024-01-01T00:00:00Z"}
```

`~/.claude/transcripts/` 下的包装转录文件仅在包含真实 Claude 使用量元数据时才会被统计。包含用户/工具事件但没有 `usage` 块的文件会被跳过，而不会进行估算。

Tokscale 的 `claude` 客户端统计的是 Claude Code 的 Token，而非 Claude Desktop 聊天。Claude Desktop 会将应用数据存储在 `~/Library/Application Support/Claude` 等位置，但 Anthropic 并未为消费级桌面聊天或聊天历史导出提供文档化、稳定的本地按消息 Token 账本。当存在 Claude Desktop 数据但只有 Claude Code JSONL 根目录可扫描时，运行 `tokscale clients` 可看到一条诊断信息。`tokscale usage` 可以根据 Claude Code 凭据尽力显示 Claude 订阅配额条，而组织/API 使用量属于 Anthropic 的 Admin Usage 和 Cost API，与本地转录扫描有意分离。

### Codex CLI

位置：`~/.codex/sessions/*.jsonl`

带 `token_count` 事件的事件驱动格式：
```json
{"type": "event_msg", "payload": {"type": "token_count", "info": {"last_token_usage": {"input_tokens": 1234, "output_tokens": 567}}}}
```

### Copilot CLI

位置：`~/.copilot/otel/*.jsonl` 或 `COPILOT_OTEL_FILE_EXPORTER_PATH` 中指定的显式路径

Copilot 支持读取文件导出的 OpenTelemetry JSONL。在运行 Copilot 之前启用它：

```bash
export COPILOT_OTEL_ENABLED=true
export COPILOT_OTEL_EXPORTER_TYPE=file
mkdir -p "$HOME/.copilot/otel"
export COPILOT_OTEL_FILE_EXPORTER_PATH="$HOME/.copilot/otel/copilot-otel-$(date +%Y%m%d-%H%M%S).jsonl"
```

PowerShell：

```powershell
$otelDir = "$HOME/.copilot/otel"
New-Item -ItemType Directory -Force -Path $otelDir | Out-Null
$env:COPILOT_OTEL_ENABLED = "true"
$env:COPILOT_OTEL_EXPORTER_TYPE = "file"
$env:COPILOT_OTEL_FILE_EXPORTER_PATH = Join-Path $otelDir ("copilot-otel-{0}.jsonl" -f (Get-Date -Format "yyyyMMdd-HHmmss"))
```

建议使用带时间戳的文件名，这样每个 Copilot 会话都会写入新文件，而不是累积到一个巨大的 OTEL 日志中。

Tokscale 将 `chat` span 作为 Token 统计的真实来源，并在第一阶段忽略工具 span 和累积指标：

```json
{"type":"span","name":"chat gpt-5.4-mini","attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"session-id","gen_ai.usage.input_tokens":1234,"gen_ai.usage.output_tokens":567,"gen_ai.usage.cache_read.input_tokens":890,"gen_ai.usage.reasoning.output_tokens":123}}
```

> Copilot 的 OTEL 负载目前不暴露稳定的工作区元数据，因此 Copilot 行可能不会显示工作区属性。Tokscale 尽可能根据报告的模型为这些行定价，不会直接信任 `github.copilot.cost`。

### Gemini CLI

位置：`$GEMINI_CLI_HOME/tmp/{projectHash}/chats/*.json`（回退：`~/.gemini/tmp/{projectHash}/chats/*.json`）

包含消息数组的会话文件：
```json
{
  "sessionId": "xxx",
  "messages": [
    {"type": "gemini", "model": "gemini-2.5-pro", "tokens": {"input": 1234, "output": 567, "cached": 890, "thoughts": 123}}
  ]
}
```

### Cursor IDE

位置：`~/.config/tokscale/cursor-cache/usage*.csv`（通过 Cursor API 同步）

Cursor 数据使用您的会话令牌从 Cursor API 获取并本地缓存。Tokscale 读取这些缓存文件来生成报告；它不会解析本地的 `~/.cursor` 会话数据。设置说明请参阅 [Cursor IDE 命令](#cursor-ide-命令)。

### Antigravity

位置：`~/.config/tokscale/antigravity-cache/sessions/*.jsonl`（通过本地 Antigravity 语言服务器 RPC 同步）

Antigravity 数据不会被根命令自动获取。请在启用了 Antigravity 的编辑器打开时运行 `tokscale antigravity sync` 来刷新本地缓存，然后对缓存的 JSONL 产物使用常规的 tokscale 报告和筛选。

### Trae

位置：`~/.config/tokscale/trae-cache/sessions/*.json`（通过官方使用量 API 同步）

Trae 数据不会被根命令自动获取。先运行一次 `tokscale trae login`，然后在生成报告前运行 `tokscale trae sync`。Tokscale 会将同步得到的 API dump 解析为会话级记录，并保留 Trae 返回的成本总额。

### Warp/Oz

位置：`~/.config/tokscale/warp-cache/usage.json`（通过已认证的 GraphQL API 同步）

Warp/Oz 数据不会被根命令自动获取。先运行 `tokscale warp login`，然后在生成报告前运行 `tokscale warp sync`。由于 Warp 不暴露基于 token 归因的本地转录，Tokscale 仅记录汇总的请求数和消费金额。

### Grok Build

位置：`$GROK_HOME/sessions/*/*/updates.jsonl`（回退：`~/.grok/sessions/*/*/updates.jsonl`）

Grok Build 数据直接从本地会话更新解析。当前日志只公开累积 `totalTokens` 计数器，没有稳定的 input/output 拆分，因此 Tokscale 将每个 turn 的正向增量记录为 input token。`grok-composer-2.5-fast` 会临时映射到 Composer 2.5 Fast 价格 override，直到专用公开价格可用。

### Jcode

位置：`$JCODE_HOME/sessions/session_*.json`（回退：`~/.jcode/sessions/session_*.json`）以及匹配的 `session_*.journal.jsonl` sidecar。

Jcode 数据直接从本地会话快照解析。Tokscale 读取助手消息的 `messages[].token_usage` 字段（`input_tokens`、`output_tokens`、`cache_read_input_tokens`、`cache_creation_input_tokens` 和 `reasoning_output_tokens`），不会伪造其他客户端的身份。匹配的 journal sidecar 会在去重前合并进同一会话流，因此在 Jcode 将其检查点写入快照之前，最近追加的消息也会被包含进来。去重使用稳定的消息 ID 进行重放去重；缺少 ID 的畸形/自定义记录则使用作用域内的回退 key。

### OpenClaw

位置：`~/.openclaw/agents/*/sessions/sessions.json`（也扫描旧版路径：`~/.clawdbot/`、`~/.moltbot/`、`~/.moldbot/`）

指向 JSONL 会话文件的索引文件：
```json
{
  "agent:main:main": {
    "sessionId": "uuid",
    "sessionFile": "/path/to/session.jsonl"
  }
}
```

包含 model_change 事件和助手消息的会话 JSONL 格式：
```json
{"type":"model_change","provider":"openai-codex","modelId":"gpt-5.2"}
{"type":"message","message":{"role":"assistant","usage":{"input":1660,"output":55,"cacheRead":108928,"cost":{"total":0.02}},"timestamp":1769753935279}}
```

### Hermes Agent

位置：`$HERMES_HOME/state.db`（回退：`~/.hermes/state.db`），以及位于 `$HERMES_HOME/profiles/*/state.db` 的标准配置文件数据库（当 `HERMES_HOME` 指向活动配置文件时，则为同级的 `~/.hermes/profiles/*/state.db`）

Hermes 将会话级使用量存储在 SQLite `sessions` 表中。Tokscale 导入 `model` 存在且 token 或费用合计非零的行，使用 `started_at` 作为时间戳，保留 `message_count`，并优先使用 `actual_cost_usd` 而非 `estimated_cost_usd`。

### Pi

位置：`~/.pi/agent/sessions/<encoded-cwd>/*.jsonl` 和 `~/.omp/agent/sessions/<encoded-cwd>/*.jsonl`（[Oh My Pi](https://github.com/can1357/oh-my-pi)）

包含会话头和消息条目的 JSONL 格式：
```json
{"type":"session","id":"pi_ses_001","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"claude-3-5-sonnet","provider":"anthropic","usage":{"input":100,"output":50,"cacheRead":10,"cacheWrite":5,"totalTokens":165}}}
```

### Kimi CLI

位置：`~/.kimi/sessions/{GROUP_ID}/{SESSION_UUID}/wire.jsonl`

包含 StatusUpdate 消息的 wire.jsonl 格式：
```json
{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983426.420942, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 1562, "output": 2463, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "chatcmpl-xxx"}}}
```

### Kimi Code

位置: `~/.kimi-code/sessions/{WORKDIR}/{SESSION_UUID}/agents/{AGENT}/wire.jsonl`
```json
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":1163,"output":352,"inputCacheRead":22272,"inputCacheCreation":0},"usageScope":"turn","time":1780410897480}
```

### Qwen CLI

位置：`~/.qwen/projects/{PROJECT_PATH}/chats/{CHAT_ID}.jsonl`

格式：JSONL — 每行一个JSON对象，每个对象包含 `type`、`model`、`timestamp`、`sessionId`、`usageMetadata` 字段。

令牌字段（来自 `usageMetadata`）：
- `promptTokenCount` → 输入令牌
- `candidatesTokenCount` → 输出令牌
- `thoughtsTokenCount` → 推理/思考令牌
- `cachedContentTokenCount` → 缓存输入令牌

### Roo Code

位置：
- 本地：`~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks/{TASK_ID}/ui_messages.json`
- 服务器（尽力而为）：`~/.vscode-server/data/User/globalStorage/rooveterinaryinc.roo-cline/tasks/{TASK_ID}/ui_messages.json`

每个任务目录可能还包含 `api_conversation_history.json`，其中包含用于模型/代理元数据的 `<environment_details>` 块。

`ui_messages.json` 是一个 UI 事件数组。Tokscale 仅计算：
- `type == "say"`
- `say == "api_req_started"`

`text` 字段是包含 Token/成本元数据的 JSON：
```json
{
  "type": "say",
  "say": "api_req_started",
  "ts": "2026-02-18T12:00:00Z",
  "text": "{\"cost\":0.12,\"tokensIn\":100,\"tokensOut\":50,\"cacheReads\":20,\"cacheWrites\":5,\"apiProtocol\":\"anthropic\"}"
}
```

### Kilo

位置：
- 本地：`~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/{TASK_ID}/ui_messages.json`
- 服务器（尽力而为）：`~/.vscode-server/data/User/globalStorage/kilocode.kilo-code/tasks/{TASK_ID}/ui_messages.json`

Kilo 使用与 Roo Code 相同的任务日志格式。Tokscale 应用相同的规则：
- 仅计算 `ui_messages.json` 中的 `say/api_req_started` 事件
- 从 `text` JSON 中解析 `tokensIn`、`tokensOut`、`cacheReads`、`cacheWrites`、`cost` 和 `apiProtocol`
- 在可用时从相邻的 `api_conversation_history.json` 中丰富模型/代理元数据

### Cline

位置：
- Linux 桌面版 VS Code：`~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/{TASK_ID}/ui_messages.json`
- macOS 桌面版 VS Code：`~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/{TASK_ID}/ui_messages.json`
- Windows 桌面版 VS Code：`%APPDATA%\Code\User\globalStorage\saoudrizwan.claude-dev\tasks\{TASK_ID}\ui_messages.json`
- 服务器（尽力而为）：`~/.vscode-server/data/User/globalStorage/saoudrizwan.claude-dev/tasks/{TASK_ID}/ui_messages.json`

Cline 是 Roo Code 和 Kilo 从中 fork 的上游项目，因此使用相同的 VS Code globalStorage 任务日志格式。Tokscale 应用相同的规则：
- 仅计算 `ui_messages.json` 中的 `say/api_req_started` 事件
- 从 `text` JSON 中解析 `tokensIn`、`tokensOut`、`cacheReads`、`cacheWrites`、`cost` 和 `apiProtocol`
- 在可用时从相邻的 `api_conversation_history.json` 中丰富模型/代理元数据

### Mux

位置:
 `~/.mux/sessions/{WORKSPACE_ID}/session-usage.json`

Mux 在 `session-usage.json` 文件中存储每个会话的累计 Token 使用量。每个文件包含一个 `byModel` 映射，其中包含每个模型的 Token 细分：
 `input`、`cached`（缓存读取）、`cacheCreate`（缓存写入）、`output`、`reasoning`
 模型名称使用 `provider:model` 格式（例如 `anthropic:claude-opus-4-6`）— tokscale 会去除 provider 前缀以识别模型
 子代理使用量由 Mux 自动汇总到父会话中，因此不存在重复计算

### Kilo CLI

位置：`~/.local/share/kilo/kilo.db`

Kilo CLI 在类似于 OpenCode 的 SQLite 数据库中存储会话数据。每条消息行包含每条消息的 Token 细分（输入、输出、缓存读取/写入、推理）以及模型和提供商归属。

### Crush

位置：通过 `$XDG_DATA_HOME/crush/projects.json` 发现的项目级 SQLite 数据库（回退路径：`~/.local/share/crush/projects.json`）

Crush 在项目级 SQLite 数据库（`crush.db`）中存储使用量。由于 Crush 不提供可靠的每条消息或每个模型的 Token 统计，Tokscale 仅导入根会话的会话级成本总计。记录显示为 `model=session-total`，Token 细分为零。

### Goose

位置：`~/.local/share/goose/sessions/sessions.db`（同时扫描 `~/Library/Application Support/goose/`、`~/Library/Application Support/Block/goose/`、`~/.local/share/Block/goose/`；可通过 `GOOSE_PATH_ROOT` 覆盖）

Goose 在 SQLite 的 `sessions.db` 中按会话保存使用量。Tokscale 从 `model_config_json` 提取模型，从 `provider_name` 提取提供商，并按会话累加输入/输出 Token 总数。当推理 Token 列有数据时也会被推断出来。

### Codebuff

位置：`~/.config/manicode/projects/<project>/chats/<chatId>/chat-messages.json`（同时扫描 `manicode-dev` 和 `manicode-staging` 频道；可通过 `CODEBUFF_DATA_DIR` 覆盖）

Codebuff（前身 Manicode）按聊天写入 JSON 文件。Tokscale 从 `metadata.usage`、`metadata.codebuff.usage` 以及 run-state 中的 `messageHistory[*].providerOptions` 回退路径解析 Token 使用量，并以反向顺序遍历历史，避免部分更新的较新条目遮蔽实际承载 Token 数的较早条目。当消息时间戳缺失时，会回退到 chat-id 目录名，最后回退到文件 mtime。

### Gajae-Code (gjc)

位置：`~/.gjc/agent/sessions/<project-slug>/*.jsonl`（可通过 `GJC_CODING_AGENT_DIR` 覆盖 agent 目录；也会解析 `GJC_CONFIG_DIR`/`PI_CONFIG_DIR` 拼接 `agent/sessions`，以及 Linux/macOS 上展平的 `$XDG_DATA_HOME/gjc/sessions/` 重定向路径）。深度为 2 的逐次子代理转录（`<slug>/<session>/N-*.jsonl`）也会被发现。

包含会话头和消息条目的 JSONL 格式。Tokscale 仅处理 assistant 消息，并在存在时优先使用 gjc 的权威性每条消息 `usage.cost.total`（USD），仅在缺失时才从 token 数重新计算：
```json
{"type":"session","id":"S1","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/work/proj"}
{"type":"message","id":"M1","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"claude-sonnet-4","provider":"anthropic","usage":{"input":1000,"output":500,"cacheRead":0,"cacheWrite":0,"totalTokens":1500,"cost":{"input":0.1,"output":0.2,"total":0.3}}}}
```

### Synthetic (synthetic.new)

Synthetic 通过后处理重归属其他来源的消息。当检测到 `hf:` 前缀模型 ID 或 `synthetic` / `glhf` / `octofriend` provider 时，消息会被归类为 `synthetic` 来源。

Tokscale 还会检测 `~/.local/share/octofriend/sqlite.db`，并在可用时解析包含 token 数据的记录。

### MiMo Code

位置：`~/.local/share/mimocode/mimocode.db`（XDG 数据目录）

MiMo Code 将会话数据存储在 SQLite 数据库中。Tokscale 查询 `message` 表并关联 `session` 表获取工作区上下文：

```sql
SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root
FROM message m
LEFT JOIN session s ON s.id = m.session_id
WHERE json_extract(m.data, '$.role') = 'assistant'
  AND json_extract(m.data, '$.tokens') IS NOT NULL
```

`data` 列为 JSON 格式，包含以下 token 相关字段：
```json
{
  "role": "assistant",
  "modelID": "claude-sonnet-4",
  "providerID": "anthropic",
  "cost": 0.0032,
  "tokens": {
    "input": 1200,
    "output": 450,
    "reasoning": 0,
    "cache": { "read": 800, "write": 0 }
  },
  "time": { "created": 1780410897000, "completed": 1780410912000 },
  "agent": "micode",
  "path": { "root": "/Users/me/project" }
}
```

Tokscale 使用时间戳、模型、provider、token 计数、成本和 agent 名称的指纹对跨 fork 会话的消息进行去重。

## 定价

Tokscale 从 [LiteLLM 的价格数据库](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json)获取实时价格。

**动态回退**：对于 LiteLLM 中尚未收录的模型（例如最近发布的模型），Tokscale 会自动从 [OpenRouter 的端点 API](https://openrouter.ai/docs/api/api-reference/endpoints/list-endpoints) 获取定价。这确保您无需等待 LiteLLM 更新，就能从模型的作者提供商（例如 glm-4.7 的 Z.AI）获得准确的价格。

**Cursor 模型定价**：对于 LiteLLM 和 OpenRouter 中都尚未收录的最新模型（例如 `gpt-5.3-codex`），Tokscale 使用从 [Cursor 模型文档](https://cursor.com/en-US/docs/models)获取的硬编码定价。这些覆盖在所有上游来源之后、模糊匹配之前检查，因此当真正的上游定价可用时会自动让步。

**Sakana Fugu 定价**：Fugu Ultra 的成本根据 Sakana 公布的按量付费（pay-as-you-go）费率估算；`fugu` 路由模型有意不定价，因为它的成本就是其所编排的底层模型的浮动费率。

**缓存**：价格数据以 1 小时 TTL 缓存到磁盘，确保快速启动：
- LiteLLM 缓存：`~/.config/tokscale/cache/pricing-litellm.json`
- OpenRouter 缓存：`~/.config/tokscale/cache/pricing-openrouter.json`（缓存支持提供商的模型作者定价信息）

定价包括：
- 输入 Token
- 输出 Token
- 缓存读取 Token（折扣）
- 缓存写入 Token
- 推理 Token（用于 o1 等模型）
- 模型专属的分层定价（例如 200k 或 272k Token 以上）

## 贡献

欢迎贡献！请按照以下步骤操作：

1. Fork 仓库
2. 创建功能分支（`git checkout -b feature/amazing-feature`）
3. 进行更改
4. 运行测试（`cd packages/core && bun run test:all`）
5. 提交更改（`git commit -m 'Add amazing feature'`）
6. 推送到分支（`git push origin feature/amazing-feature`）
7. 打开 Pull Request

### 开发指南

- 遵循现有代码风格
- 为新功能添加测试
- 根据需要更新文档
- 保持提交集中和原子化

## 致谢

- 感谢 [ccusage](https://github.com/ryoppippi/ccusage)、[viberank](https://github.com/sculptdotfun/viberank) 和 [Isometric Contributions](https://github.com/jasonlong/isometric-contributions) 提供的灵感
- [Ratatui](https://github.com/ratatui/ratatui) 终端 UI 框架
- [Solid.js](https://www.solidjs.com/) 响应式渲染
- [LiteLLM](https://github.com/BerriAI/litellm) 价格数据
- [napi-rs](https://napi.rs/) Rust/Node.js 绑定
- [github-contributions-canvas](https://github.com/sallar/github-contributions-canvas) 2D 图表参考

## 许可证

<p align="center">
  <a href="https://github.com/junhoyeo">
    <img src=".github/assets/labtocat-on-spaceship.png" width="540">
  </a>
</p>

<p align="center">
  <strong>MIT © <a href="https://github.com/junhoyeo">Junho Yeo</a></strong>
</p>

如果您觉得这个项目有趣，**请考虑给它一个星标 ⭐** 或 [在 GitHub 上关注我](https://github.com/junhoyeo) 加入旅程（已有 1.1k+ 人加入）。我全天候编程，定期发布令人惊叹的东西——您的支持不会白费。
