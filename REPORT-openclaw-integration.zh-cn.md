> **关于 Tokscale** | [junhoyeo/tokscale](https://github.com/junhoyeo/tokscale) 是一款追踪 AI 编程助手 Token 使用量的开源工具，支持 **14 个主流客户端**，包括 Claude Code、Cursor IDE、Gemini CLI、Codex CLI、OpenCode、OpenClaw、Amp、Droid、Pi、Kimi CLI、Qwen CLI、Roo Code、Kilo 以及 Synthetic。自发布以来已在 GitHub 上获得 **近 1,000 颗星**（991 ⭐），全球用户累计追踪的 Token 总量已**超过 5,000 亿**。5,000 亿 Token 意味着什么？这相当于价值数百万美元的 AI 算力。Tokscale 帮助开发者将这些「隐性成本」可视化。 | 🔗 [GitHub](https://github.com/junhoyeo/tokscale) · [官网](https://tokscale.ai) · [中文 README](https://github.com/junhoyeo/tokscale/blob/main/README.zh-cn.md)

---

# Tokscale 如何集成 OpenClaw：一个面向四次更名客户端的有状态 JSONL 解析器

> **日期**：2026 年 3 月 3 日
> **资料来源**：Git 历史记录、GitHub releases/PRs、代码库分析

---

## 目录

- [概述](#概述)
- [1. 什么是 OpenClaw？](#1-什么是-openclaw)
- [2. OpenClaw 是如何被添加的](#2-openclaw-是如何被添加的)
- [3. 数据格式深度解析](#3-数据格式深度解析)
  - [3.1 事件类型](#31-事件类型)
  - [3.2 Token 字段](#32-token-字段)
  - [3.3 旧版索引格式](#33-旧版索引格式)
- [4. 解析器架构](#4-解析器架构)
  - [4.1 入口点](#41-入口点)
  - [4.2 有状态解析循环](#42-有状态解析循环)
  - [4.3 Session ID 解析](#43-session-id-解析)
  - [4.4 时间戳回退机制](#44-时间戳回退机制)
- [5. 客户端注册表条目](#5-客户端注册表条目)
- [6. 扫描器：一个客户端，四条路径](#6-扫描器一个客户端四条路径)
  - [6.1 品牌更名历史](#61-品牌更名历史)
  - [6.2 代码实现](#62-代码实现)
- [7. 跨代码库的集成面](#7-跨代码库的集成面)
- [8. OpenClaw 与其他客户端的区别](#8-openclaw-与其他客户端的区别)
  - [8.1 有状态 vs 自包含](#81-有状态-vs-自包含)
  - [8.2 对比表](#82-对比表)
  - [8.3 关键架构要点](#83-关键架构要点)
- [9. 测试覆盖](#9-测试覆盖)
- [10. 参考资料](#10-参考资料)

---

## 概述

[OpenClaw](https://openclaw.ai/) 是一个基于终端的 AI 编程代理，经历了四次名称变更：**Clawd → Moltbot → Moldbot → OpenClaw**。Tokscale 自 [v1.2.0](https://github.com/junhoyeo/tokscale/releases/tag/v1.2.0)（2026 年 1 月 30 日）起支持该客户端，通过 [PR #139](https://github.com/junhoyeo/tokscale/pull/139) 添加，并以 🦞 龙虾 emoji 作为标志发布。

在 tokscale 支持的 14 个客户端中，OpenClaw 在架构上具有两个独特之处：

1. **有状态解析** — 它是唯一使用 `model_change` 事件为后续消息设置上下文的客户端。所有其他客户端都直接在每条消息中嵌入模型元数据。
2. **四条扫描路径** — Tokscale 扫描 `~/.openclaw/`、`~/.clawdbot/`、`~/.moltbot/` 和 `~/.moldbot/`，以统一所有品牌更名期间的使用历史。没有其他客户端需要超过两条扫描路径。

解析器实现位于一个 362 行的 Rust 文件中（[`openclaw.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs)），包含 8 个单元测试，覆盖模型切换、用户消息过滤、缺失模型上下文、会话索引解析和基于文件名的会话 ID 推导。

---

## 1. 什么是 OpenClaw？

OpenClaw 是一个基于终端的 AI 编程代理 — 概念上类似于 Claude Code、Codex CLI 或 Gemini CLI，但独立开发。它将会话数据以 JSONL 文件格式存储在 `~/.openclaw/agents/` 目录下。

该项目经历了多次更名：

| 名称 | 目录 | 时期 |
|------|------|------|
| **Clawd** | `~/.clawdbot/` | 初始版本 |
| **Moltbot** | `~/.moltbot/` | 第一次更名 |
| **Moldbot** | `~/.moldbot/` | 第二次更名 |
| **OpenClaw** | `~/.openclaw/` | 当前版本（至少自 2026 年 1 月起） |

每次更名都改变了工具的数据目录，但 JSONL 格式保持不变。在任何一个名称下安装和使用过该工具的用户，都在对应目录中拥有会话文件。

---

## 2. OpenClaw 是如何被添加的

OpenClaw 支持作为旗舰版本在 tokscale v1.2.0 中发布：

| 详情 | 值 |
|------|-----|
| **PR** | [#139](https://github.com/junhoyeo/tokscale/pull/139) |
| **版本** | v1.2.0 |
| **发布日期** | 2026 年 1 月 30 日 |
| **Release** | [v1.2.0](https://github.com/junhoyeo/tokscale/releases/tag/v1.2.0) |
| **公告** | 🦞 `tokscale@v1.2.0` is here!（现已支持 [OpenClaw](https://github.com/openclaw/openclaw)） |

v1.2.0 的发布标题包含了 🦞 龙虾 emoji，标志着这是一个重要的客户端新增。旧版路径支持（Clawd/Moltbot/Moldbot）在提交 [`6659dde`](https://github.com/junhoyeo/tokscale/commit/6659dde) 中引入。

---

## 3. 数据格式深度解析

### 3.1 事件类型

OpenClaw 会话是 JSONL 文件（每行一个 JSON 对象），采用**有状态事件流**模型。有两种事件类型：

**`model_change`** — 为后续消息设置当前活跃的模型和提供商：

```jsonl
{"type":"model_change","provider":"openai-codex","modelId":"gpt-5.2"}
```

**`message`** — 记录一次交互及其 token 使用量：

```jsonl
{"type":"message","message":{"role":"assistant","usage":{"input":1660,"output":55,"cacheRead":108928,"cost":{"total":0.02}},"timestamp":1769753935279}}
```

只有 `role: "assistant"` 的消息会被计数。用户消息被完全跳过。

有状态性是其核心特征：一个 `model_change` 事件设置的上下文将应用于**所有后续的 `message` 事件**，直到下一个 `model_change`。例如：

```jsonl
{"type":"model_change","provider":"openai-codex","modelId":"gpt-5.2"}
{"type":"message","message":{"role":"assistant","usage":{"input":100,"output":50},"timestamp":1700000000000}}
{"type":"message","message":{"role":"assistant","usage":{"input":200,"output":100},"timestamp":1700000001000}}
{"type":"model_change","provider":"anthropic","modelId":"claude-3.5-sonnet"}
{"type":"message","message":{"role":"assistant","usage":{"input":300,"output":150},"timestamp":1700000002000}}
```

解析结果为：
- 消息 1 → `gpt-5.2`，通过 `openai-codex`（100 输入 / 50 输出）
- 消息 2 → `gpt-5.2`，通过 `openai-codex`（200 输入 / 100 输出）
- 消息 3 → `claude-3.5-sonnet`，通过 `anthropic`（300 输入 / 150 输出）

如果一个 `message` 事件出现在任何 `model_change` 之前，它将被**静默丢弃**（没有模型上下文可分配）。这在 [`test_parse_openclaw_session_no_model_change`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs) 测试中有验证。

### 3.2 Token 字段

| Tokscale 字段 | OpenClaw JSON 键 | 说明 |
|---------------|------------------|------|
| `input` | `usage.input` | 消耗的输入 token |
| `output` | `usage.output` | 生成的输出 token |
| `cache_read` | `usage.cacheRead` | 读取的缓存输入 token |
| `cache_write` | `usage.cacheWrite` | 写入缓存的 token |
| `reasoning` | N/A | 硬编码为 `0` — OpenClaw 格式不包含推理 token |
| `cost` | `usage.cost.total` | 文件中存在，但 tokscale 会通过 LiteLLM **重新计算**以保持跨客户端一致性 |

所有 token 值在缺失时默认为 `0`，并通过 `.max(0)` 钳制为非负数：

```rust
// crates/tokscale-core/src/sessions/openclaw.rs, 第 197-202 行
TokenBreakdown {
    input: usage.input.unwrap_or(0).max(0),
    output: usage.output.unwrap_or(0).max(0),
    cache_read: usage.cache_read.unwrap_or(0).max(0),
    cache_write: usage.cache_write.unwrap_or(0).max(0),
    reasoning: 0,
}
```

`totalTokens` 字段被反序列化但未使用（`#[allow(dead_code)]`）。

### 3.3 旧版索引格式

除了直接解析 JSONL 文件外，解析器还支持 **`sessions.json` 索引文件** — 一个会话条目的 JSON 映射：

```json
{
  "agent:main:main": {
    "sessionId": "abc-123",
    "sessionFile": "/absolute/path/to/session.jsonl"
  },
  "agent:feature:branch": {
    "sessionId": "def-456",
    "sessionFile": "relative-session.jsonl"
  }
}
```

`sessionFile` 字段可以是：
- **绝对路径** — 直接使用
- **相对路径** — 相对于 `sessions.json` 所在目录解析
- **缺失/为空** — 回退到同一目录下的 `{sessionId}.jsonl`

这由 `resolve_session_path()` 处理：

```rust
// crates/tokscale-core/src/sessions/openclaw.rs, 第 102-119 行
fn resolve_session_path(index_dir: &Path, entry: &SessionEntry) -> PathBuf {
    match entry.session_file.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(session_file) => {
            let path = Path::new(session_file);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                index_dir.join(path)
            }
        }
        None => index_dir.join(format!("{}.jsonl", entry.session_id)),
    }
}
```

---

## 4. 解析器架构

### 4.1 入口点

解析器暴露两个公共函数：

| 函数 | 输入 | 使用场景 |
|------|------|----------|
| `parse_openclaw_transcript(path)` | 直接的 `.jsonl` 文件路径 | 主要路径 — 扫描器找到 `*.jsonl` 文件 |
| `parse_openclaw_index(path)` | `sessions.json` 索引文件 | 旧版兼容 — 解析引用的会话文件 |

两者最终都调用内部函数 `parse_openclaw_session(path, session_id)`。

### 4.2 有状态解析循环

核心解析器（[`parse_openclaw_session`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs#L121-L212)）维护两个可变状态：

```rust
let mut current_model: Option<String> = None;
let mut current_provider: Option<String> = None;
```

它通过 `BufReader` 逐行读取文件，并根据 `entry_type` 进行分派：

```rust
match entry.entry_type.as_str() {
    "model_change" => {
        // 更新 current_model 和 current_provider
        if let Some(model) = entry.model_id {
            current_model = Some(model);
        }
        if let Some(provider) = entry.provider {
            current_provider = Some(provider);
        }
    }
    "message" => {
        // 仅处理具有 usage 数据且已知模型的 assistant 消息
        // 跳过条件：role != "assistant"、无 usage 数据、或尚未出现 model_change
    }
    _ => {} // 未知类型静默忽略
}
```

解析器使用 `simd_json` 进行高性能反序列化，跨行复用字节缓冲区：

```rust
let mut buffer = Vec::with_capacity(4096);
// ...
buffer.clear();
buffer.extend_from_slice(trimmed.as_bytes());
let entry: OpenClawEntry = match simd_json::from_slice(&mut buffer) { ... };
```

### 4.3 Session ID 解析

- **直接文件**：Session ID 从文件名主干推导（例如 `my-session-123.jsonl` → `"my-session-123"`）
- **基于索引**：Session ID 来自 `sessions.json` 中的 `sessionId` 字段

### 4.4 时间戳回退机制

如果消息缺少 `timestamp` 字段，解析器会回退到文件的修改时间：

```rust
let file_mtime_ms = std::fs::metadata(session_path)
    .and_then(|m| m.modified())
    .ok()
    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    .map(|d| d.as_millis() as i64)
    .unwrap_or(0);

// 在消息处理时：
let timestamp = msg.timestamp.unwrap_or(file_mtime_ms);
```

这是 OpenClaw 独有的 — 其他客户端要么始终有时间戳，要么使用其他回退策略。

---

## 5. 客户端注册表条目

OpenClaw 在集中式 `define_clients!` 宏中注册为 `ClientId::OpenClaw = 7`：

```rust
// crates/tokscale-core/src/clients.rs, 第 167-174 行
OpenClaw = 7 => {
    id: "openclaw",
    root: PathRoot::Home,
    relative: ".openclaw/agents",
    pattern: "*.jsonl",
    headless: false,
    parse_local: true
},
```

关键属性：
- **`root: PathRoot::Home`** — 基目录为 `$HOME`
- **`relative: ".openclaw/agents"`** — 主扫描目录
- **`pattern: "*.jsonl"`** — 递归匹配所有 JSONL 文件
- **`headless: false`** — 不支持无头/CI 模式
- **`parse_local: true`** — 解析器处理本地文件解析（相对于 Cursor 的 API 同步方式）

---

## 6. 扫描器：一个客户端，四条路径

### 6.1 品牌更名历史

OpenClaw 拥有 tokscale 所有客户端中最多的扫描路径 — **4 个目录**，全部作为 `ClientId::OpenClaw` 扫描：

```
~/.openclaw/agents/**/*.jsonl     ← 当前（OpenClaw）
~/.clawdbot/agents/**/*.jsonl     ← 旧版（Clawd）
~/.moltbot/agents/**/*.jsonl      ← 旧版（Moltbot）
~/.moldbot/agents/**/*.jsonl      ← 旧版（Moldbot）
```

作为对比：
- 大多数客户端有 **1** 条扫描路径
- Codex 有 **3** 条（sessions + archived_sessions + headless）
- RooCode 和 KiloCode 各有 **2** 条（本地 + vscode-server）
- OpenClaw 有 **4** 条 — 所有客户端中最多

四条路径产生的文件都归属于 `ClientId::OpenClaw`。一个从 Clawd 开始使用，经历 Moltbot 和 Moldbot 升级，最终使用 OpenClaw 的用户，将在 TUI 中看到其全部使用历史统一显示在"OpenClaw"客户端下。

### 6.2 代码实现

来自 [`crates/tokscale-core/src/scanner.rs` 第 226–256 行](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/scanner.rs#L226-L256)：

```rust
if enabled.contains(&ClientId::OpenClaw) {
    // OpenClaw 文件：~/.openclaw/agents/**/*.jsonl
    let openclaw_path = ClientId::OpenClaw.data().resolve_path(home_dir);
    tasks.push((ClientId::OpenClaw, openclaw_path, ClientId::OpenClaw.data().pattern));

    // 旧版路径（Clawd -> Moltbot -> OpenClaw 品牌更名历史）
    let clawdbot_path = format!("{}/.clawdbot/agents", home_dir);
    tasks.push((ClientId::OpenClaw, clawdbot_path, ClientId::OpenClaw.data().pattern));

    let moltbot_path = format!("{}/.moltbot/agents", home_dir);
    tasks.push((ClientId::OpenClaw, moltbot_path, ClientId::OpenClaw.data().pattern));

    let moldbot_path = format!("{}/.moldbot/agents", home_dir);
    tasks.push((ClientId::OpenClaw, moldbot_path, ClientId::OpenClaw.data().pattern));
}
```

四个扫描任务使用相同的模式（`*.jsonl`）和相同的客户端 ID（`ClientId::OpenClaw`）。扫描通过 Rayon（`into_par_iter`）并行执行，结果合并到 `ScanResult` 中的单个 `Vec<PathBuf>`。

---

## 7. 跨代码库的集成面

OpenClaw 在 Rust 核心、CLI 和前端的 **10 个文件**中被引用：

### 核心层（Rust）

| 文件 | 角色 | 关键细节 |
|------|------|----------|
| [`crates/tokscale-core/src/sessions/openclaw.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs) | 解析器 | 362 行，8 个测试，有状态 `model_change` 处理 |
| [`crates/tokscale-core/src/clients.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/clients.rs) | 注册表 | `ClientId::OpenClaw = 7`，路径 `~/.openclaw/agents`，模式 `*.jsonl` |
| [`crates/tokscale-core/src/scanner.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/scanner.rs) | 扫描器 | 4 条扫描路径（第 226–256 行），Rayon 并行执行 |

### CLI 层（Rust）

| 文件 | 角色 | 关键细节 |
|------|------|----------|
| [`crates/tokscale-cli/src/main.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-cli/src/main.rs) | CLI 标志 | 所有报告命令的 `--openclaw` 过滤标志 |
| [`crates/tokscale-cli/src/tui/client_ui.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-cli/src/tui/client_ui.rs) | TUI 显示 | 显示名称 `"OpenClaw"`，快捷键 `'8'` |
| [`crates/tokscale-cli/src/commands/wrapped.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-cli/src/commands/wrapped.rs) | Wrapped 图像 | 用于年度回顾图像生成的 Logo URL |

### 前端层（TypeScript）

| 文件 | 角色 | 关键细节 |
|------|------|----------|
| [`packages/frontend/src/lib/types.ts`](https://github.com/junhoyeo/tokscale/blob/main/packages/frontend/src/lib/types.ts) | 类型定义 | `ClientType` 联合类型包含 `"openclaw"` |
| [`packages/frontend/src/lib/constants.ts`](https://github.com/junhoyeo/tokscale/blob/main/packages/frontend/src/lib/constants.ts) | UI 常量 | 显示名称、Logo 路径、颜色 `#EF4444`（红色） |
| [`packages/frontend/src/lib/validation/submission.ts`](https://github.com/junhoyeo/tokscale/blob/main/packages/frontend/src/lib/validation/submission.ts) | 验证 | 排行榜提交的有效客户端白名单 |

---

## 8. OpenClaw 与其他客户端的区别

### 8.1 有状态 vs 自包含

根本性的架构区别在于**模型身份的追踪方式**：

- **所有其他 JSONL 客户端**（Claude Code、Codex、Pi、Kimi、Qwen）：每条消息都携带自己的 `model` 字段。解析器可以独立处理任何一行。
- **OpenClaw**：模型身份由 `model_change` 事件设置，并持续作用于后续的 `message` 事件。解析器必须顺序处理各行并维护状态。

这意味着 OpenClaw 的解析器**无法**在行级别上并行化（但通过 Rayon 实现了文件级别的并行处理）。它是一个流式解析器，而非 map-reduce 解析器。

### 8.2 对比表

| 方面 | OpenClaw | Claude Code | Codex CLI | Gemini CLI |
|------|----------|-------------|-----------|------------|
| **格式** | 带有状态 `model_change` 事件的 JSONL | 每条消息带 `model` 字段的 JSONL | 带 `token_count` 事件的 JSONL | 包含消息数组的 JSON |
| **模型追踪** | 有状态 — `model_change` 为后续消息设置上下文 | 每条消息的 `model` 字段 | 每个事件的 model 字段 | 每条消息的 `model` 字段 |
| **文件中的费用** | `usage.cost.total`（tokscale 重新计算） | 否 | 否 | 否 |
| **缓存 token** | `cacheRead`、`cacheWrite` | `cache_read_input_tokens` | 否 | `cached` |
| **推理 token** | 否 | 否 | 否 | `thoughts` |
| **旧版路径** | 3 条旧版路径（Clawd、Moltbot、Moldbot） | 无 | `archived_sessions/` | 无 |
| **无头模式支持** | 否 | 否 | 是 | 否 |
| **Provider 字段** | 显式（来自 `model_change`） | 推断 | 推断 | 隐式 |
| **去重机制** | 无 | `dedup_key` | 无 | 无 |
| **会话发现** | 文件名主干或 `sessions.json` 索引 | 基于目录 | 基于目录 | 项目哈希 |
| **时间戳回退** | 文件修改时间（如 `timestamp` 缺失） | 不需要 | 不需要 | 不需要 |

### 8.3 关键架构要点

1. **有状态解析** — OpenClaw 是唯一使用有状态流的客户端。如果一个 `message` 事件在任何 `model_change` 之前到达，它会被静默丢弃（无模型可分配）。这是设计使然 — 解析器采取保守策略，从不猜测。

2. **无去重机制** — 不同于 Claude Code（使用 `dedup_key` 避免重复计数）和 OpenCode（通过 SQLite 去重），OpenClaw 没有内置去重机制。解析器信任每行 JSONL 代表一个唯一事件。

3. **费用重新计算** — OpenClaw 是唯一在会话文件中嵌入费用的客户端（`usage.cost.total`）。然而，tokscale 读取该值后会通过 LiteLLM 定价**重新计算**，以保持跨客户端一致性。文件中嵌入的费用仅作为参考，不用于最终报告。

4. **无 Agent 追踪** — OpenClaw 消息的 `agent` 字段始终为 `None`。JSONL 格式包含每个条目的 `id` 字段，但这不是持久的 agent 身份标识 — 它只是行标识符。解析器不提取也不使用它。

5. **Provider 的显式性** — OpenClaw 是少数具有显式 `provider` 字段（通过 `model_change` 设置）的客户端之一。大多数其他客户端要么从模型名称推断 provider，要么根本不追踪。如果未设置 provider，解析器默认使用 `"unknown"`。

---

## 9. 测试覆盖

解析器在 [`openclaw.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs#L214-L362) 中有 8 个单元测试：

| 测试 | 验证内容 |
|------|----------|
| `test_parse_openclaw_session_with_model_change` | 基本流程：`model_change` → `message` → 正确的模型、provider、token、费用 |
| `test_parse_openclaw_session_user_messages_ignored` | 用户角色的消息被跳过；仅计算 assistant |
| `test_parse_openclaw_session_no_model_change` | `model_change` 之前的消息被丢弃（返回空） |
| `test_parse_openclaw_transcript_derives_session_id_from_filename` | `my-session-123.jsonl` → session ID `"my-session-123"` |
| `test_parse_openclaw_index_absolute_session_file` | 索引中带绝对 `sessionFile` 路径 → 正确解析 |
| `test_parse_openclaw_index_relative_session_file` | 索引中带相对 `sessionFile` → 与索引目录拼接 |
| `test_parse_openclaw_index_missing_session_file_fallback` | 索引中无 `sessionFile` → 回退到 `{sessionId}.jsonl` |

扫描器也有一个 OpenClaw 专属测试：

| 测试（scanner.rs） | 验证内容 |
|---------------------|----------|
| `test_scan_all_clients_openclaw_jsonl_only` | 扫描 `~/.openclaw/agents/` 并仅找到 `*.jsonl` 文件（而非 `sessions.json`） |

---

## 10. 参考资料

### PR 和提交

| 参考 | 描述 |
|------|------|
| [PR #139](https://github.com/junhoyeo/tokscale/pull/139) | 原始 OpenClaw 支持（v1.2.0） |
| [v1.2.0 Release](https://github.com/junhoyeo/tokscale/releases/tag/v1.2.0) | 🦞 发布公告 |
| [`6659dde`](https://github.com/junhoyeo/tokscale/commit/6659dde) | 旧版路径支持（Clawd/Moltbot/Moldbot） |
| [PR #237](https://github.com/junhoyeo/tokscale/pull/237) | `define_clients!` 宏（集中式客户端注册表） |
| [PR #230](https://github.com/junhoyeo/tokscale/pull/230) | `source` → `client` 术语重命名 |

### 源文件

| 文件 | 行数 | 用途 |
|------|------|------|
| [`crates/tokscale-core/src/sessions/openclaw.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs) | 362 | 解析器实现 + 8 个测试 |
| [`crates/tokscale-core/src/clients.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/clients.rs) | 392 | 客户端注册表（`ClientId::OpenClaw = 7`） |
| [`crates/tokscale-core/src/scanner.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/scanner.rs) | 851 | 文件扫描器（4 条 OpenClaw 路径，第 226–256 行） |

### 外部资源

| 资源 | URL |
|------|-----|
| OpenClaw | https://openclaw.ai/ |
| Tokscale GitHub | https://github.com/junhoyeo/tokscale |
| LiteLLM（定价数据源） | https://github.com/BerriAI/litellm |

---

*基于代码库分析、Git 历史记录和 PR 审查生成。所有文件路径、行号和代码片段均已与 tokscale 仓库核实。*

---

## 🔗 链接

| | |
|---|---|
| **GitHub** | https://github.com/junhoyeo/tokscale |
| **官方网站** | https://tokscale.ai |
| **中文 README** | https://github.com/junhoyeo/tokscale/blob/main/README.zh-cn.md |
| **npm** | https://www.npmjs.com/package/tokscale |

---

## ⭐ 支持作者

如果这篇文章对您有帮助，欢迎留下您的意见和反馈！

- 🌟 给 Tokscale 点一颗星 → [**Star on GitHub**](https://github.com/junhoyeo/tokscale)
- 👤 在 GitHub 上关注 [@junhoyeo](https://github.com/junhoyeo)

后续我将继续分享 AI 工具、开源项目和性能优化相关的内容。您的支持是我持续创作的动力！🙏

---

**版权声明**：本文为 [@junhoyeo](https://github.com/junhoyeo) 的原创文章，遵循 CC 4.0 BY-SA 版权协议，转载请附上原文出处链接及本声明。
