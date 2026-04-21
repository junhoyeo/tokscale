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

| 图标 | 客户端 | 数据位置 | 支持状态 |
|------|----------|---------------|-----------|
| <img width="48px" src=".github/assets/client-opencode.png" alt="OpenCode" /> | [OpenCode](https://github.com/sst/opencode) | `~/.local/share/opencode/opencode.db` (1.2+，包含 `opencode-stable.db` 等所有渠道) 或 `~/.local/share/opencode/storage/message/` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-claude.jpg" alt="Claude" /> | [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `~/.claude/projects/` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-openclaw.jpg" alt="OpenClaw" /> | [OpenClaw](https://openclaw.ai/) | `~/.openclaw/agents/` (+ 旧版: `.clawdbot`, `.moltbot`, `.moldbot`) | ✅ 支持 |
| <img width="48px" src=".github/assets/client-openai.jpg" alt="Codex" /> | [Codex CLI](https://github.com/openai/codex) | `~/.codex/sessions/` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-hermes.png" alt="Hermes Agent" /> | [Hermes Agent](https://github.com/NousResearch/hermes-agent) | `$HERMES_HOME/state.db`（回退：`~/.hermes/state.db`） | ✅ 支持 |
| <img width="48px" src=".github/assets/client-gemini.png" alt="Gemini" /> | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `~/.gemini/tmp/*/chats/*.json` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-cursor.jpg" alt="Cursor" /> | [Cursor IDE](https://cursor.com/) | 通过 `~/.config/tokscale/cursor-cache/` API 同步 | ✅ 支持 |
| <img width="48px" src=".github/assets/client-amp.png" alt="Amp" /> | [Amp (AmpCode)](https://ampcode.com/) | `~/.local/share/amp/threads/` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-droid.png" alt="Droid" /> | [Droid (Factory Droid)](https://factory.ai/) | `~/.factory/sessions/` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-pi.png" alt="Pi" /> | [Pi](https://github.com/badlogic/pi-mono) | `~/.pi/agent/sessions/` and `~/.omp/agent/sessions/` ([Oh My Pi](https://github.com/can1357/oh-my-pi)) | ✅ 支持 |
| <img width="48px" src=".github/assets/client-kimi.png" alt="Kimi" /> | [Kimi CLI](https://github.com/MoonshotAI/kimi-cli) | `~/.kimi/sessions/` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-qwen.png" alt="Qwen" /> | [Qwen CLI](https://github.com/QwenLM/qwen-cli) | `~/.qwen/projects/` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-roocode.png" alt="Roo Code" /> | [Roo Code](https://github.com/RooCodeInc/Roo-Code) | `~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks/` (+ server: `~/.vscode-server/data/User/globalStorage/rooveterinaryinc.roo-cline/tasks/`) | ✅ 支持 |
| <img width="48px" src=".github/assets/client-kilocode.png" alt="Kilo" /> | [Kilo](https://github.com/Kilo-Org/kilocode) | `~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/` (+ server: `~/.vscode-server/data/User/globalStorage/kilocode.kilo-code/tasks/`) | ✅ 支持 |
| <img width="48px" src=".github/assets/client-mux.png" alt="Mux" /> | [Mux](https://github.com/coder/mux) | `~/.mux/sessions/` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-kilocode.png" alt="Kilo CLI" /> | [Kilo CLI](https://github.com/nicepkg/kilo) | `~/.local/share/kilo/kilo.db` | ✅ 支持 |
| <img width="48px" src=".github/assets/client-crush.png" alt="Crush" /> | [Crush](https://crush.ai/) | `$XDG_DATA_HOME/crush/projects.json`（项目注册表；回退路径：`~/.local/share/crush/projects.json`） | ✅ 支持 |
| <img width="48px" src=".github/assets/client-synthetic.png" alt="Synthetic" /> | [Synthetic](https://synthetic.new/) | 通过 `hf:` 模型前缀或 `synthetic` provider 从其他来源重归属（+ [Octofriend](https://github.com/synthetic-lab/octofriend): `~/.local/share/octofriend/sqlite.db`） | ✅ 支持 |

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
  - [Cursor IDE 命令](#cursor-ide-命令)
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
  - 4 个交互式视图：概览、模型、每日、统计
  - 键盘和鼠标导航
  - 9 种颜色主题的 GitHub 风格贡献图
  - 实时筛选和排序
  - 零闪烁渲染
- **多平台支持** - 跟踪 OpenCode、Claude Code、Codex CLI、Cursor IDE、Gemini CLI、Amp、Droid、OpenClaw、Hermes Agent、Pi、Kimi CLI、Qwen CLI、Roo Code、Kilo、Mux、Kilo CLI、Crush 和 Synthetic 的使用情况
- **实时定价** - 从 LiteLLM 获取当前价格，带 1 小时磁盘缓存；OpenRouter 自动回退和新模型的 Cursor 定价支持
- **详细分解** - 输入、输出、缓存读写和推理 Token 跟踪
- **原生 Rust 核心** - 所有解析和聚合在 Rust 中完成，处理速度提升 10 倍
- **Web 可视化** - 带 2D 和 3D 视图的交互式贡献图
- **灵活筛选** - 按平台、日期范围或年份筛选
- **导出为 JSON** - 为外部可视化工具生成数据
- **社交平台** - 分享使用情况、排行榜竞争、查看公开个人资料

## 安装

### 快速开始

```bash
# 直接用 npx 运行
npx tokscale@latest

# 或使用 bunx
bunx tokscale@latest

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

- **6 个视图**：概览（图表 + 热门模型）、模型、每日、每时、统计（贡献图）、代理
- **键盘导航**：
  - `1-6` 或 `←/→/Tab`：切换视图
  - `↑/↓`：导航列表
  - `c/d/t`：按成本/日期/Token 排序
  - `s`：打开来源选择对话框
  - `g`：打开分组方式选择对话框（模型、客户端+模型、客户端+提供商+模型）
  - `p`：循环 9 种颜色主题
  - `r`：刷新数据
  - `e`：导出为 JSON
  - `q`：退出
- **鼠标支持**：点击标签、按钮和筛选器
- **主题**：Green、Halloween、Teal、Blue、Pink、Purple、Orange、Monochrome、YlGnBu
- **设置持久化**：偏好设置保存到 `~/.config/tokscale/settings.json`（参见[配置](#配置)）

### 分组策略

在 TUI 中按 `g` 或在 `--light`/`--json` 模式下使用 `--group-by` 来控制模型行的聚合方式：

| 策略 | 标志 | TUI 默认 | 效果 |
|------|------|---------|------|
| **模型** | `--group-by model` | ✅ | 每个模型一行 — 合并所有客户端和提供商 |
| **客户端 + 模型** | `--group-by client,model` | | 每个客户端-模型对一行 |
| **客户端 + 提供商 + 模型** | `--group-by client,provider,model` | | 最详细 — 不合并 |

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

### 按平台筛选

```bash
# 仅显示 OpenCode 使用量
tokscale --opencode

# 仅显示 Claude Code 使用量
tokscale --claude

# 仅显示 Codex CLI 使用量
tokscale --codex

# 仅显示 OpenClaw 使用量
tokscale --openclaw

# 仅显示 Pi 使用量
tokscale --pi

# 仅显示 Gemini CLI 使用量
tokscale --gemini

# 仅显示 Cursor IDE 使用量（需要先 `tokscale cursor login`）
tokscale --cursor

# 仅显示 Amp 使用量
tokscale --amp

# 仅显示 Droid 使用量
tokscale --droid

# 仅显示 Hermes Agent 使用量
tokscale --hermes

# 仅显示 Kimi CLI 使用量
tokscale --kimi

# 仅显示 Qwen CLI 使用量
tokscale --qwen

# 仅显示 Roo Code 使用量
tokscale --roocode

# 仅显示 Kilo 使用量
tokscale --kilocode

# 仅显示 Mux 使用量
tokscale --mux

# 仅显示 Kilo CLI 使用量
tokscale --kilo

# 仅显示 Crush 使用量
tokscale --crush

# 仅显示 Synthetic (synthetic.new) 使用量
tokscale --synthetic

# 组合筛选
tokscale --opencode --claude
```

### 日期筛选

日期筛选器适用于所有生成报告的命令（`tokscale`、`tokscale models`、`tokscale monthly`、`tokscale graph`）：

```bash
# 快速日期快捷方式
tokscale --today              # 仅今天
tokscale --week               # 最近 7 天
tokscale --month              # 本月

# 自定义日期范围（包含，本地时区）
tokscale --since 2024-01-01 --until 2024-12-31

# 按年份筛选
tokscale --year 2024

# 与其他选项组合
tokscale models --week --claude --json
tokscale monthly --month --benchmark
```

> **注意**：日期筛选器使用本地时区。`--since` 和 `--until` 都是包含的。

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
```

**查询策略：**

价格查询使用多步解析策略：

1. **精确匹配** - 在 LiteLLM/OpenRouter 数据库中直接查找
2. **别名解析** - 解析友好名称（例如：`big-pickle` → `glm-4.7`）
3. **层级后缀剥离** - 移除质量层级（`gpt-5.2-xhigh` → `gpt-5.2`）
4. **版本标准化** - 处理版本格式（`claude-3-5-sonnet` ↔ `claude-3.5-sonnet`）
5. **提供商前缀匹配** - 尝试常见前缀（`anthropic/`、`openai/` 等）
6. **Cursor 模型定价** - LiteLLM/OpenRouter 中尚未收录的模型的硬编码定价（例如：`gpt-5.3-codex`）
7. **模糊匹配** - 部分模型名称的词边界匹配

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

# 查看当前登录用户
tokscale whoami

# 提交使用量数据到排行榜
tokscale submit

# 带筛选提交
tokscale submit --opencode --claude --since 2024-01-01

# 预览将要提交的内容（试运行）
tokscale submit --dry-run

# 登出
tokscale logout
```

<img alt="CLI Submit" src="./.github/assets/cli-submit.png" />

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

### 示例输出（`--light` 版本）

<img alt="CLI Light" src="./.github/assets/cli-light.png" />

### 配置

Tokscale 将设置存储在 `~/.config/tokscale/settings.json`：

```json
{
  "colorPalette": "blue",
  "includeUnusedModels": false
}
```

| 设置 | 类型 | 默认值 | 描述 |
|---------|------|---------|-------------|
| `colorPalette` | string | `"blue"` | TUI 颜色主题（green、halloween、teal、blue、pink、purple、orange、monochrome、ylgnbu） |
| `includeUnusedModels` | boolean | `false` | 在报告中显示零 Token 的模型 |
| `autoRefreshEnabled` | boolean | `false` | 在 TUI 中启用自动刷新 |
| `autoRefreshMs` | number | `60000` | 自动刷新间隔（30000-3600000ms） |
| `nativeTimeoutMs` | number | `300000` | 原生子进程处理最大时间（5000-3600000ms） |

### 环境变量

环境变量会覆盖配置文件中的值。适用于 CI/CD 或一次性使用：

| 变量 | 默认值 | 描述 |
|----------|---------|-------------|
| `TOKSCALE_NATIVE_TIMEOUT_MS` | `300000`（5 分钟） | 覆盖 `nativeTimeoutMs` 配置 |

```bash
# 示例：为非常大的数据集增加超时时间
TOKSCALE_NATIVE_TIMEOUT_MS=600000 tokscale graph --output data.json
```

> **注意**：如需永久更改，建议在 `~/.config/tokscale/settings.json` 中设置 `nativeTimeoutMs`。环境变量适用于一次性覆盖或 CI/CD。

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
- **来源筛选**：按平台筛选（OpenCode、Claude、Codex、Cursor、Gemini、Amp、Droid、OpenClaw、Hermes Agent、Pi、Kimi、Qwen、Roo Code、Kilo、Mux、Kilo CLI、Crush、Synthetic）
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

- 将 `<username>` 替换为您的 GitHub 用户名
- 可选查询参数：
  - `theme=light` 使用浅色主题
  - `sort=tokens`（默认）或 `sort=cost` 控制排名依据
  - `compact=1` 使用紧凑布局 + 紧凑数字表示法（例如 `1.2M`、`$3.4K`）
- 示例：
  - `https://tokscale.ai/api/embed/<username>/svg?theme=light&sort=cost&compact=1`

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

1. **登录** - 运行 `tokscale login` 通过 GitHub 认证
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
tokscale graph --opencode --claude

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

| 平台 | 架构 | 状态 |
|----------|--------------|--------|
| macOS | x86_64 | ✅ 支持 |
| macOS | aarch64（Apple Silicon） | ✅ 支持 |
| Linux | x86_64（glibc） | ✅ 支持 |
| Linux | aarch64（glibc） | ✅ 支持 |
| Linux | x86_64（musl） | ✅ 支持 |
| Linux | aarch64（musl） | ✅ 支持 |
| Windows | x86_64 | ✅ 支持 |
| Windows | aarch64 | ✅ 支持 |

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
| Hermes Agent | `~/.hermes/` | `%USERPROFILE%\.hermes\` | 可通过 `HERMES_HOME` 环境变量配置（[源码](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/session-storage.md)） |
| Gemini CLI | `~/.gemini/` | `%USERPROFILE%\.gemini\` | 所有平台使用相同路径 |
| Amp | `~/.local/share/amp/` | `%USERPROFILE%\.local\share\amp\` | 与 OpenCode 一样使用 `xdg-basedir` |
| Cursor | API 同步 | API 同步 | 通过 API 获取数据，缓存在 `%USERPROFILE%\.config\tokscale\cursor-cache\` |
| Droid | `~/.factory/` | `%USERPROFILE%\.factory\` | 所有平台使用相同路径 |
| Pi | `~/.pi/` and `~/.omp/` | `%USERPROFILE%\.pi\` and `%USERPROFILE%\.omp\` | 所有平台使用相同路径（支持 Pi 和 [Oh My Pi](https://github.com/can1357/oh-my-pi)） |
| Kimi CLI | `~/.kimi/` | `%USERPROFILE%\.kimi\` | 所有平台使用相同路径 |
| Qwen CLI | `~/.qwen/` | `%USERPROFILE%\.qwen\` | 所有平台使用相同路径 |
| Roo Code | `~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks/` | `%USERPROFILE%\.config\Code\User\globalStorage\rooveterinaryinc.roo-cline\tasks\` | VS Code globalStorage 任务日志 |
| Kilo | `~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/` | `%USERPROFILE%\.config\Code\User\globalStorage\kilocode.kilo-code\tasks\` | VS Code globalStorage 任务日志 |
| Mux | `~/.mux/sessions/` | `%USERPROFILE%\.mux\sessions\` | 所有平台相同路径 |
| Kilo CLI | `~/.local/share/kilo/` | `%USERPROFILE%\.local\share\kilo\` | 与 OpenCode 一样使用 `xdg-basedir` |
| Crush | `$XDG_DATA_HOME/crush/`（回退路径：`~/.local/share/crush/`） | `%USERPROFILE%\.local\share\crush\`（如果设置了 `%XDG_DATA_HOME%`，则为 `%XDG_DATA_HOME%\crush\`） | 使用带回退路径的 XDG 数据目录 |
| Synthetic | 从其他来源重归属 | 从其他来源重归属 | 检测 `hf:` 模型前缀 + `synthetic` provider |

> **注意**：在 Windows 上，`~` 扩展为 `%USERPROFILE%`（例如 `C:\Users\用户名`）。这些工具故意使用 Unix 风格的路径（如 `.local/share`）而不是 Windows 原生路径（如 `%APPDATA%`），以实现跨平台一致性。

#### Windows 特定配置

Tokscale 将配置存储在：
- **配置**: `%USERPROFILE%\.config\tokscale\settings.json`
- **缓存**: `%USERPROFILE%\.cache\tokscale\`
- **Cursor 凭据**: `%USERPROFILE%\.config\tokscale\cursor-credentials.json`

## 会话数据保留

默认情况下，一些 AI 编程助手会自动删除旧的会话文件。为了准确跟踪，请禁用或延长清理周期以保留使用历史。

| 平台 | 默认值 | 配置文件 | 禁用设置 | 来源 |
|----------|---------|-------------|-------------------|--------|
| Claude Code | **⚠️ 30 天** | `~/.claude/settings.json` | `"cleanupPeriodDays": 9999999999` | [文档](https://docs.anthropic.com/en/docs/claude-code/settings) |
| Gemini CLI | 禁用 | `~/.gemini/settings.json` | `"general.sessionRetention.enabled": false` | [文档](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/session-management.md) |
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

如果您已启用清理并想禁用它，请在 `~/.gemini/settings.json` 中删除或设置 `enabled: false`：
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

位置：`~/.claude/projects/{projectPath}/*.jsonl`

包含使用数据的助手消息的 JSONL 格式：
```json
{"type": "assistant", "message": {"model": "claude-sonnet-4-20250514", "usage": {"input_tokens": 1234, "output_tokens": 567, "cache_read_input_tokens": 890}}, "timestamp": "2024-01-01T00:00:00Z"}
```

### Codex CLI

位置：`~/.codex/sessions/*.jsonl`

带 `token_count` 事件的事件驱动格式：
```json
{"type": "event_msg", "payload": {"type": "token_count", "info": {"last_token_usage": {"input_tokens": 1234, "output_tokens": 567}}}}
```

### Gemini CLI

位置：`~/.gemini/tmp/{projectHash}/chats/*.json`

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

位置：`~/.config/tokscale/cursor-cache/`（通过 Cursor API 同步）

Cursor 数据使用您的会话令牌从 Cursor API 获取并本地缓存。运行 `tokscale cursor login` 进行认证。设置说明请参阅 [Cursor IDE 命令](#cursor-ide-命令)。

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

位置：`$HERMES_HOME/state.db`（回退：`~/.hermes/state.db`）

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

### Synthetic (synthetic.new)

Synthetic 通过后处理重归属其他来源的消息。当检测到 `hf:` 前缀模型 ID 或 `synthetic` / `glhf` / `octofriend` provider 时，消息会被归类为 `synthetic` 来源。

Tokscale 还会检测 `~/.local/share/octofriend/sqlite.db`，并在可用时解析包含 token 数据的记录。

## 定价

Tokscale 从 [LiteLLM 的价格数据库](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json)获取实时价格。

**动态回退**：对于 LiteLLM 中尚未收录的模型（例如最近发布的模型），Tokscale 会自动从 [OpenRouter 的端点 API](https://openrouter.ai/docs/api/api-reference/endpoints/list-endpoints) 获取定价。

**Cursor 模型定价**：对于 LiteLLM 和 OpenRouter 中都尚未收录的最新模型（例如 `gpt-5.3-codex`），Tokscale 使用从 [Cursor 模型文档](https://cursor.com/en-US/docs/models)获取的硬编码定价。这些覆盖在所有上游来源之后、模糊匹配之前检查，因此当真正的上游定价可用时会自动让步。

**缓存**：价格数据以 1 小时 TTL 缓存到磁盘，确保快速启动：
- LiteLLM 缓存：`~/.cache/tokscale/pricing-litellm.json`
- OpenRouter 缓存：`~/.cache/tokscale/pricing-openrouter.json`（缓存支持提供商的模型作者定价信息）

定价包括：
- 输入 Token
- 输出 Token
- 缓存读取 Token（折扣）
- 缓存写入 Token
- 推理 Token（用于 o1 等模型）
- 分层定价（200k Token 以上）

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
