<!-- <CENTERED SECTION FOR GITHUB DISPLAY> -->

<div align="center">

[![Tokscale](./.github/assets/hero-v2.png)](https://tokscale.ai)

</div>

> 여러 플랫폼에서 AI 코딩 어시스턴트의 **토큰 사용량과 비용**을 추적하는 고성능 CLI 도구 및 시각화 대시보드

> [!TIP]
>
> **v2 출시 — 네이티브 Rust TUI, 크로스 플랫폼 지원 등.** <br />
> 저는 매주 새로운 오픈소스 프로젝트를 공개합니다. 놓치지 마세요.
>
> | [<img alt="GitHub Follow" src="https://img.shields.io/github/followers/junhoyeo?style=flat-square&logo=github&labelColor=black&color=24292f" width="156px" />](https://github.com/junhoyeo) | GitHub에서 [@junhoyeo](https://github.com/junhoyeo)를 팔로우하고 더 많은 프로젝트를 만나보세요. AI, 인프라 등 다양한 분야를 다룹니다. |
> | :-----| :----- |
> [<img alt="Discord link" src="https://img.shields.io/discord/1480206352755458110?color=5865F2&label=discord&labelColor=black&logo=discord&logoColor=white&style=flat-square" width="156px" />](https://discord.gg/h6DUGWdBbm) | [Discord](https://discord.gg/h6DUGWdBbm)에서 함께해요 — 세계 최고의 바이버들과 어울리세요. |

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

> **[`bunx tokscale submit`](#소셜-플랫폼-명령어)를 실행하여 사용량 데이터를 리더보드에 제출하고 공개 프로필을 만드세요!**

## 개요

**Tokscale**은 아래 플랫폼들의 **토큰 소비량을 수집하고 분석**해 한 눈에 볼 수 있도록 해 줍니다.

| 로고 | 클라이언트 | 데이터 위치 |
|------|----------|---------------|
| <img width="48px" src=".github/assets/client-opencode.png" alt="OpenCode" /> | [OpenCode](https://github.com/sst/opencode) | `~/.local/share/opencode/opencode.db` (1.2+, `opencode-stable.db` 등 모든 채널 포함) 또는 `~/.local/share/opencode/storage/message/` |
| <img width="48px" src=".github/assets/client-claude.jpg" alt="Claude" /> | [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `~/.claude/projects/` 및 `~/.claude/transcripts/` |
| <img width="48px" src=".github/assets/client-openclaw.jpg" alt="OpenClaw" /> | [OpenClaw](https://openclaw.ai/) | `~/.openclaw/agents/` (+ 레거시: `.clawdbot`, `.moltbot`, `.moldbot`) |
| <img width="48px" src=".github/assets/client-openai.jpg" alt="Codex" /> | [Codex CLI](https://github.com/openai/codex) | `~/.codex/sessions/` |
| <img width="48px" src=".github/assets/client-sakana.png" alt="Sakana Fugu" /> | [Sakana Fugu](https://sakana.ai/fugu/) | Codex를 통해 추적 — `~/.codex/sessions/*.jsonl` (`model_provider: sakana`) |
| <img width="48px" src=".github/assets/client-copilot.jpg" alt="Copilot" /> | [GitHub Copilot CLI](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/use-the-github-copilot-coding-agent-in-cli) | `~/.copilot/otel/*.jsonl` (+ `COPILOT_OTEL_FILE_EXPORTER_PATH`) |
| <img width="48px" src=".github/assets/client-hermes.png" alt="Hermes Agent" /> | [Hermes Agent](https://github.com/NousResearch/hermes-agent) | `$HERMES_HOME/state.db` 및 `$HERMES_HOME/profiles/*/state.db` (폴백: `~/.hermes/...`) |
| <img width="48px" src=".github/assets/client-gemini.png" alt="Gemini" /> | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `$GEMINI_CLI_HOME/tmp/*/chats/*.json` (폴백: `~/.gemini/tmp/*/chats/*.json`) |
| <img width="48px" src=".github/assets/client-cursor.jpg" alt="Cursor" /> | [Cursor IDE](https://cursor.com/) | Cursor API 내보내기를 `~/.config/tokscale/cursor-cache/usage*.csv`에 캐싱 (`~/.cursor` 아님) |
| <img width="48px" src=".github/assets/client-amp.png" alt="Amp" /> | [Amp (AmpCode)](https://ampcode.com/) | `~/.local/share/amp/threads/` |
| <img width="48px" src=".github/assets/client-codebuff.png" alt="Codebuff" /> | [Codebuff](https://codebuff.com/) | `~/.config/manicode/` (+ `manicode-dev`, `manicode-staging`; `CODEBUFF_DATA_DIR`로 오버라이드 가능) |
| <img width="48px" src=".github/assets/client-droid.png" alt="Droid" /> | [Droid (Factory Droid)](https://factory.ai/) | `~/.factory/sessions/` |
| <img width="48px" src=".github/assets/client-pi.png" alt="Pi" /> | [Pi](https://github.com/badlogic/pi-mono) | `~/.pi/agent/sessions/` and `~/.omp/agent/sessions/` ([Oh My Pi](https://github.com/can1357/oh-my-pi)) |
| <img width="48px" src=".github/assets/client-kimi.png" alt="Kimi" /> | [Kimi CLI](https://github.com/MoonshotAI/kimi-cli) / [Kimi Code](https://github.com/MoonshotAI/kimi-code) | kimi-cli: `~/.kimi/sessions/` kimi-code: `~/.kimi-code/sessions/` (override via `KIMI_CODE_HOME`) |
| <img width="48px" src=".github/assets/client-qwen.png" alt="Qwen" /> | [Qwen CLI](https://github.com/QwenLM/qwen-cli) | `~/.qwen/projects/` |
| <img width="48px" src=".github/assets/client-roocode.png" alt="Roo Code" /> | [Roo Code](https://github.com/RooCodeInc/Roo-Code) | `~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks/` (+ server: `~/.vscode-server/data/User/globalStorage/rooveterinaryinc.roo-cline/tasks/`) |
| <img width="48px" src=".github/assets/client-kilocode.png" alt="Kilo" /> | [Kilo](https://github.com/Kilo-Org/kilocode) | `~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/` (+ server: `~/.vscode-server/data/User/globalStorage/kilocode.kilo-code/tasks/`) |
| <img width="48px" src=".github/assets/client-kilocode.png" alt="Kilo CLI" /> | [Kilo CLI](https://github.com/nicepkg/kilo) | `~/.local/share/kilo/kilo.db` |
| <img width="48px" src=".github/assets/client-mux.png" alt="Mux" /> | [Mux](https://github.com/coder/mux) | `~/.mux/sessions/` |
| <img width="48px" src=".github/assets/client-crush.png" alt="Crush" /> | [Crush](https://crush.ai/) | `$XDG_DATA_HOME/crush/projects.json` (프로젝트 레지스트리, 기본값: `~/.local/share/crush/projects.json`) |
| <img width="48px" src=".github/assets/client-goose.png" alt="Goose" /> | [Goose](https://github.com/aaif-goose/goose) | `~/.local/share/goose/sessions/sessions.db` (+ macOS Application Support, 레거시 Block/goose 경로; `GOOSE_PATH_ROOT`로 오버라이드 가능) |
| <img width="48px" src=".github/assets/client-antigravity.png" alt="Antigravity" /> | [Google Antigravity](https://antigravity.google/) | `tokscale antigravity sync`로 `~/.config/tokscale/antigravity-cache/sessions/*.jsonl`에 캐싱 (로컬 언어 서버 RPC 사용) |
| <img width="48px" src=".github/assets/client-antigravity.png" alt="Antigravity CLI" /> | [Antigravity CLI](https://antigravity.google/) | `~/.gemini/antigravity-cli/conversations/*.db` (`GEMINI_CLI_HOME`로 Gemini 홈 경로 오버라이드 가능; 로컬 SQLite를 직접 읽으므로 `antigravity sync`가 필요 없음) |
| <img width="48px" src=".github/assets/client-trae.png" alt="Trae" /> | [Trae IDE](https://www.trae.ai/) / [Trae Solo](https://www.trae.ai/solo) (국제판) | `tokscale trae sync`로 `~/.config/tokscale/trae-cache/sessions/*.json`에 캐싱 (공식 API의 계정 단위 사용량) |
| <img width="48px" src="https://github.com/warpdotdev.png" alt="Warp" /> | [Warp](https://www.warp.dev/) / Oz | `tokscale warp sync`로 `~/.config/tokscale/warp-cache/usage.json`에 캐싱 (집계된 요청 수 및 비용만; 토큰 트랜스크립트 없음) |
| <img width="48px" src="https://github.com/xai-org.png" alt="Grok Build" /> | Grok Build | `$GROK_HOME/sessions/*/*/updates.jsonl` (폴백: `~/.grok/sessions/*/*/updates.jsonl`) |
| <img width="48px" src=".github/assets/client-zed.webp" alt="Zed Agent" /> | [Zed Agent](https://zed.dev/docs/ai/agent-panel) | `~/.local/share/zed/threads/threads.db` (macOS: `~/Library/Application Support/Zed/threads/threads.db`; Windows: `%LOCALAPPDATA%/Zed/threads/threads.db`; 호스팅된 Zed 모델 전용, 외부 ACP 에이전트 제외) |
| <img width="48px" src="https://github.com/kirodotdev.png" alt="Kiro" /> | Kiro | `~/.kiro/sessions/cli/*.json` (+ `*.jsonl`), `~/.local/share/kiro-cli/data.sqlite3` (macOS: `~/Library/Application Support/kiro-cli/data.sqlite3`), 그리고 Kiro IDE globalStorage 스냅샷 (`Kiro/User/globalStorage/kiro.kiroagent`; macOS Application Support, Linux `~/.config/Kiro`, Windows `%APPDATA%\Kiro`) |
| <img width="48px" src="https://github.com/cline.png" alt="Cline" /> | [Cline](https://github.com/cline/cline) | VS Code globalStorage tasks (Linux: `~/.config/Code/...`; macOS: `~/Library/Application Support/Code/...`; Windows: `%APPDATA%\Code\...`; server: `~/.vscode-server/data/User/globalStorage/saoudrizwan.claude-dev/tasks/`) |
| <img width="48px" src="https://github.com/user-attachments/assets/7246e920-f3f8-4b6e-847e-030ae04e86c2" alt="Gajae-Code" /> | [gajae-code (gjc)](https://github.com/Yeachan-Heo/gajae-code) | `~/.gjc/agent/sessions/` (`GJC_CODING_AGENT_DIR`, `GJC_CONFIG_DIR`, `PI_CONFIG_DIR`로 오버라이드 가능; Linux/macOS에서는 `$XDG_DATA_HOME/gjc/sessions/`도 확인) |
| <img width="48px" src=".github/assets/client-jcode.png" alt="Jcode" /> | [Jcode](https://github.com/1jehuang/jcode) | `~/.jcode/sessions/session_*.json` + `session_*.journal.jsonl` 사이드카 (`JCODE_HOME`으로 재정의 가능) |
| <img width="48px" src="https://github.com/XiaomiMiMo.png" alt="MiMo Code" /> | [MiMo Code](https://github.com/XiaomiMiMo/MiMo-Code) | `~/.local/share/mimocode/mimocode.db` (XDG 데이터 디렉토리; SQLite) |
| <img width="48px" src="https://github.com/JetBrains.png" alt="Junie" /> | [Junie](https://www.jetbrains.com/junie/) | `~/.junie/sessions/*/events.jsonl` |
| <img width="48px" src="https://raw.githubusercontent.com/CommandCodeAI/command-code/main/.github/commandcode/logo/command-code-logo-black-bg.png" alt="Command Code" /> | [Command Code](https://github.com/CommandCodeAI/command-code) | `~/.commandcode/projects/**/*.jsonl` (토큰 사용량은 트랜스크립트에서 토큰당 약 4자 기준으로 추정; 디스크에 저장되지 않음) |
| <img width="48px" src="https://github.com/zai-org.png" alt="ZCode" /> | [ZCode](https://zcode.z.ai/) | `~/.zcode/cli/db/db.sqlite`(v2 사용량 데이터베이스) 및 `~/.zcode/projects/**/*.jsonl`(레거시 기록) |
| <img width="48px" src="https://github.com/alibaba.png" alt="OpenCodeReview" /> | [OpenCodeReview](https://github.com/alibaba/open-code-review) | `~/.opencodereview/sessions/**/*.jsonl` |
| <img width="48px" src="https://pc3.gtimg.com/softmgr/logo/48/43068_48_1764842447.png" alt="CodeBuddy" /> | [CodeBuddy](https://www.codebuddy.cn/docs/cli/overview) (CLI, IDE, VS Code 플러그인) | `~/.codebuddy/projects/**/*.jsonl` + 확장 프로그램 로그 |
| <img width="48px" src="https://static.workbuddy.cn/web/agents/008054d6beaaf4a83e2d049e982e1244560726dc/assets/share-logo.png" alt="WorkBuddy" /> | WorkBuddy | `~/.workbuddy/projects/**/*.jsonl` + SQLite 폴백 |
| <img width="48px" src=".github/assets/client-devin.jpg" alt="Devin CLI" /> | [Devin CLI](https://devin.ai/) | `~/.local/share/devin/cli/sessions.db` (SQLite) |
| <img width="48px" src=".github/assets/client-devin.jpg" alt="Devin Desktop" /> | [Devin Desktop](https://devin.ai/) | ACP 이벤트: macOS `~/Library/Application Support/Devin/User/acp-events/`; Linux `~/.config/Devin/User/acp-events/`; Windows `%APPDATA%\Devin\User\acp-events\` |
| <img width="48px" src=".github/assets/client-synthetic.png" alt="Synthetic" /> | [Synthetic](https://synthetic.new/) | `hf:` 모델/`synthetic` provider 감지로 다른 소스에서 재귀속 (+ [Octofriend](https://github.com/synthetic-lab/octofriend): `~/.local/share/octofriend/sqlite.db`) |

[🚅 LiteLLM의 가격 데이터](https://github.com/BerriAI/litellm)를 사용해 **실시간 비용 계산**을 제공합니다. 구간별 가격 모델(대용량 컨텍스트 등)과 **캐시 토큰 할인**도 지원합니다.

### 왜 "Tokscale"인가요?

이 프로젝트는 **[카르다쇼프 척도(Kardashev Scale)](https://ko.wikipedia.org/wiki/%EC%B9%B4%EB%A5%B4%EB%8B%A4%EC%87%BC%ED%94%84_%EC%B2%99%EB%8F%84)**에서 영감을 받았습니다. 카르다쇼프 척도는 문명의 기술 수준을 **에너지 소비량**으로 분류합니다. 유형 I 문명은 행성에서 사용 가능한 모든 에너지를 활용하고, 유형 II는 항성의 전체 출력을 포착하며, 유형 III는 은하 전체의 에너지를 통제합니다.

AI 지원 개발 시대에 **토큰은 새로운 에너지**입니다. 토큰은 우리의 사고력을 구동하고, 생산성을 높이며, 창의적 결과물을 이끌어냅니다. 카르다쇼프 척도가 우주적 규모에서 에너지 소비를 추적하듯, Tokscale은 AI 증강 개발의 단계를 올라가며 **토큰 소비를 측정하고 시각화**합니다. 가볍게 쓰는 사용자든 매일 수백만 개의 토큰을 소비하는 파워 유저든, Tokscale은 "내가 어디에서 무엇을 얼마나 쓰고 있는지"를 분명하게 보여줍니다.

## 목차

- [개요](#개요)
  - [왜 "Tokscale"인가요?](#왜-tokscale인가요)
- [기능](#기능)
- [설치](#설치)
  - [빠른 시작](#빠른-시작)
  - [사전 요구사항](#사전-요구사항)
  - [개발 환경 설정](#개발-환경-설정)
  - [네이티브 모듈 빌드](#네이티브-모듈-빌드)
- [사용법](#사용법)
  - [기본 명령어](#기본-명령어)
  - [TUI 기능](#tui-기능)
  - [플랫폼별 필터링](#플랫폼별-필터링)
  - [날짜 필터링](#날짜-필터링)
  - [가격 조회](#가격-조회)
  - [사용자 정의 가격 오버라이드](#사용자-정의-가격-오버라이드)
  - [소셜 플랫폼 명령어](#소셜-플랫폼-명령어)
  - [Autosubmit](#autosubmit)
  - [Cursor IDE 명령어](#cursor-ide-명령어)
  - [Antigravity 명령어](#antigravity-명령어)
  - [Trae 명령어](#trae-명령어)
  - [Warp/Oz 명령어](#warpoz-명령어)
  - [작업 기반 리포트](#작업-기반-리포트)
  - [구독 사용량](#구독-사용량)
  - [예시 출력](#예시-출력---light-버전)
  - [설정](#설정)
  - [환경 변수](#환경-변수)
- [프론트엔드 시각화](#프론트엔드-시각화)
  - [기능](#기능-1)
  - [프론트엔드 실행](#프론트엔드-실행)
- [소셜 플랫폼](#소셜-플랫폼)
  - [기능](#기능-2)
  - [GitHub 프로필 임베드 위젯](#github-프로필-임베드-위젯)
  - [GitHub 프로필 뱃지](#github-프로필-뱃지)
  - [시작하기](#시작하기)
  - [데이터 검증](#데이터-검증)
- [Wrapped 2025](#wrapped-2025)
  - [명령어](#명령어)
  - [포함 내용](#포함-내용)
- [개발](#개발)
  - [사전 요구사항](#사전-요구사항-1)
  - [실행 방법](#실행-방법)
- [지원 플랫폼](#지원-플랫폼)
  - [네이티브 모듈 타겟](#네이티브-모듈-타겟)
  - [Windows 지원](#windows-지원)
- [세션 데이터 보존](#세션-데이터-보존)
- [데이터 소스](#데이터-소스)
- [가격](#가격)
- [기여](#기여)
  - [개발 가이드라인](#개발-가이드라인)
- [감사의 글](#감사의-글)
- [라이선스](#라이선스)

## 기능

- **인터랙티브 TUI 모드** - Ratatui 기반의 터미널 UI (기본 모드)
  - 6개 인터랙티브 뷰: 개요, 모델, 일별, 시간별, 통계, 에이전트 (선택적 Minutely 뷰는 `minutelyTabEnabled`로 활성화)
  - 키보드 및 마우스 지원
  - 설정 가능한 색상 테마의 GitHub 스타일 기여 그래프
  - 실시간 필터링 및 정렬
  - 깜빡임 없는 렌더링
- **멀티 플랫폼 지원** - OpenCode, Claude Code, Codex CLI, Copilot CLI, Cursor IDE, Gemini CLI, Amp, Codebuff, Droid, OpenClaw, Hermes Agent, Pi, Kimi CLI, Qwen CLI, Roo Code, Kilo, Mux, Kilo CLI, Crush, Goose, Antigravity, Antigravity CLI, Zed, Kiro, Trae, Warp/Oz, Cline, Gajae-Code, Grok Build, Jcode, MiMo Code, Command Code, Junie, ZCode, OpenCodeReview, CodeBuddy, WorkBuddy, Devin CLI, Devin Desktop, Synthetic 사용량 통합 추적
- **실시간 가격 반영** - LiteLLM에서 최신 가격을 가져와(디스크 캐시 1시간) 비용 계산; OpenRouter 자동 폴백 및 신규 모델용 Cursor 가격 지원
- **상세 분석** - 입력, 출력, 캐시 읽기/쓰기, 추론 토큰까지 추적
- **네이티브 Rust 코어** - 모든 파싱과 집계를 Rust로 처리해 최대 10배 빠른 성능
- **웹 시각화** - 2D 및 3D 뷰의 인터랙티브 기여 그래프
- **유연한 필터링** - 플랫폼, 날짜 범위 또는 연도별 필터링
- **작업 기반 리포트** - LLM 기반 세션 요약 및 작업 그룹화, 여러 백엔드 지원 (Apple FM, Claude, Codex, Gemini, Kiro)
- **JSON 내보내기** - 외부 시각화 도구/자동화용 데이터 생성
- **소셜 플랫폼** - 사용량 공유, 리더보드 경쟁, 공개 프로필 조회

## 설치

### 빠른 시작

```bash
# npx로 바로 실행
npx tokscale@latest

# 또는 bunx 사용
bunx tokscale@latest

# 별칭 설치 없이 Deno 사용
deno x npm:tokscale@latest

# 라이트 모드 (테이블 렌더링만)
npx tokscale@latest --light
```

이게 전부입니다! 별도 설정 없이 바로 완전한 인터랙티브 TUI 경험을 제공합니다.

> **패키지 구조**: `tokscale`은 `@tokscale/cli`를 설치하는 별칭 패키지입니다 ([`swc`](https://www.npmjs.com/package/swc)처럼). 둘 다 네이티브 Rust 코어 (`@tokscale/core`)가 포함된 동일한 CLI를 설치합니다.

### 사전 요구사항

- [Node.js](https://nodejs.org/) 또는 [Bun](https://bun.sh/)
- (선택) 소스에서 네이티브 모듈을 빌드하려면 Rust 툴체인

### 개발 환경 설정

로컬 개발 또는 소스에서 빌드하는 경우:

```bash
# 저장소 클론
git clone https://github.com/junhoyeo/tokscale.git
cd tokscale

# Bun 설치 (아직 설치하지 않은 경우)
curl -fsSL https://bun.sh/install | bash

# 의존성 설치
bun install

# 개발 모드에서 CLI 실행
bun run cli
```

> **참고**: `bun run cli`는 로컬 개발용입니다. `bunx tokscale`로 설치하면 명령이 직접 실행됩니다. 아래 사용법 섹션은 설치된 바이너리 명령을 보여줍니다.

### 네이티브 모듈 빌드

네이티브 Rust 모듈은 CLI 동작에 **필수**입니다. 병렬 파일 스캐닝과 SIMD JSON 파싱을 통해 처리 속도를 약 10배 향상시킵니다:

```bash
# 네이티브 코어 빌드 (저장소 루트에서 실행)
bun run build:core
```

> **참고**: `bunx tokscale@latest`로 설치하면 네이티브 바이너리가 사전 빌드되어 포함됩니다. 소스에서 빌드는 로컬 개발 시에만 필요합니다.

## 사용법

### 기본 명령어

```bash
# 인터랙티브 TUI 실행 (기본)
tokscale

# 특정 탭으로 TUI 실행
tokscale models    # 모델 탭
tokscale monthly   # 일별 뷰 (일별 분석 표시)

# 레거시 CLI 테이블 출력 사용
tokscale --light
tokscale models --light

# 명시적으로 TUI 실행
tokscale tui

# 기여 그래프 데이터를 JSON으로 내보내기
tokscale graph --output data.json

# JSON으로 데이터 출력 (스크립팅/자동화용)
tokscale --json                    # 기본 모델 뷰를 JSON으로
tokscale models --json             # 모델 분석을 JSON으로
tokscale monthly --json            # 월별 분석을 JSON으로
tokscale models --json > report.json   # 파일로 저장
```

### TUI 기능

인터랙티브 TUI 모드는 다음을 제공합니다:

- **8개 뷰**: 개요 (차트 + 상위 모델), Usage (구독 할당량), 모델, 일별, 시간별, 통계 (기여 그래프), 에이전트. 분 단위 뷰(Minutely)는 기본적으로 숨겨져 있으며 `settings.json`의 `minutelyTabEnabled`로 활성화할 수 있습니다 — [설정](#설정) 참조
- **키보드 내비게이션**:
  - `←/→/Tab/BackTab`: 뷰 전환
  - `↑/↓` 또는 `Home/End`: 목록 탐색
  - `Enter`: 일별 상세 보기 (Daily 탭) / 그래프 셀 선택 (Stats 탭)
  - `Esc` 또는 `Backspace`: 다이얼로그 닫기 / 상세 보기 나가기
  - `c/d/t`: 비용/날짜/토큰별 정렬
  - `j`: 오늘로 이동
  - `s`: 소스 선택 다이얼로그 열기
  - `g`: 그룹 기준 선택 다이얼로그 열기 (모델, 클라이언트+모델, 클라이언트+프로바이더+모델, 워크스페이스+모델, 세션+모델, 클라이언트+세션+모델)
  - `h`: Daily/Hourly 차트 단위 전환 (Overview 탭)
  - `v`: Table/Profile 뷰 전환 (Hourly 탭)
  - `y`: 선택된 행을 클립보드에 복사
  - `p`: 색상 테마 순환
  - `r`: 데이터 새로고침; `Shift+R`로 자동 새로고침 토글; `+`/`-`로 간격 조정
  - `e`: JSON으로 내보내기
  - `q` 또는 `Ctrl+C`: 종료
- **마우스 지원**: 탭, 버튼, 필터 클릭
- **테마**: Green, Halloween, Teal, Blue, Pink, Purple, Orange, Monochrome, YlGnBu, Graphite, Lagoon, Dusk
- **설정 저장**: 설정이 `~/.config/tokscale/settings.json`에 저장됨 ([설정](#설정) 참조)

### 그룹 기준 전략

TUI에서 `g`를 누르거나 `--light`/`--json` 모드에서 `--group-by`를 사용하여 모델 행 집계 방식을 제어합니다:

| 전략 | 플래그 | TUI 기본값 | 효과 |
|------|--------|-----------|------|
| **모델** | `--group-by model` | ✅ | 모델당 한 행 — 모든 클라이언트와 프로바이더 병합 |
| **클라이언트 + 모델** | `--group-by client,model` | | 클라이언트-모델 쌍당 한 행 |
| **클라이언트 + 프로바이더 + 모델** | `--group-by client,provider,model` | | 가장 세분화 — 병합 없음 |
| **워크스페이스 + 모델** | `--group-by workspace,model` | | 로컬 사용량을 워크스페이스 키별로, 그 다음 모델별로 그룹화 |
| **세션 + 모델** | `--group-by session,model` | | `session_id`와 모델당 한 행 — 특정 에이전트-CLI 세션에 비용 귀속 |
| **클라이언트 + 세션 + 모델** | `--group-by client,session,model` | | 클라이언트, 세션, 모델당 한 행 — `session_id`로 조인하는 멀티 에이전트 러너에 유용 |

**`--group-by model`** (가장 통합)

| 클라이언트 | 프로바이더 | 모델 | 비용 |
|-----------|-----------|------|------|
| OpenCode, Claude, Amp | github-copilot, anthropic | claude-opus-4-5 | $2,424 |
| OpenCode, Claude | anthropic, github-copilot | claude-sonnet-4-5 | $1,332 |

**`--group-by client,model`** (CLI 기본값)

| 클라이언트 | 프로바이더 | 모델 | 비용 |
|-----------|-----------|------|------|
| OpenCode | github-copilot, anthropic | claude-opus-4-5 | $1,368 |
| Claude | anthropic | claude-opus-4-5 | $970 |

**`--group-by client,provider,model`** (가장 세분화)

| 클라이언트 | 프로바이더 | 모델 | 비용 |
|-----------|-----------|------|------|
| OpenCode | github-copilot | claude-opus-4-5 | $1,200 |
| OpenCode | anthropic | claude-opus-4-5 | $168 |
| Claude | anthropic | claude-opus-4-5 | $970 |

**`--group-by session,model`** (세션별 비용 귀속)

`tokscale models --json --group-by session,model`은 `(session_id, model)`당 하나의 항목을 출력합니다. 각 항목은 최상위 `sessionId` 필드를 포함하므로, 다운스트림 도구(예: 멀티 에이전트 IDE)가 비용 데이터를 특정 에이전트-CLI 세션에 다시 조인할 수 있습니다:

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

모든 행에 클라이언트 이름도 필요하다면 `--group-by client,session,model`을 사용하세요 (20개 이상 지원되는 모든 CLI에 걸친 단일 스폰).

### 플랫폼별 필터링

`--client` (단축형 `-c`) 플래그로 하나 이상의 클라이언트로 리포트 범위를 좁힐 수 있습니다. 반복 사용 가능하며 콤마로 구분된 값도 지원하고, 모든 리포트 명령에서 동작합니다:

```bash
# OpenCode 사용량만 표시
tokscale --client opencode

# 콤마로 구분: 여러 클라이언트 동시 필터
tokscale --client opencode,claude

# 반복: 같은 효과 (쉘 alias와 함께 쓰기 좋음)
tokscale -c opencode -c claude

# Cursor IDE는 Tokscale의 API 캐시를 사용; 먼저 login + sync --json 실행
tokscale --client cursor

# Synthetic (synthetic.new) 은 다른 에이전트 세션에서 검출됨
tokscale --client synthetic

# 다른 필터와 조합
tokscale --client opencode,claude --week --json
```

가능한 값: `opencode`, `claude`, `codex`, `copilot`, `gemini`, `cursor`, `amp`, `codebuff`, `droid`, `openclaw`, `hermes`, `pi`, `kimi`, `qwen`, `roocode`, `kilocode`, `kilo`, `mux`, `crush`, `goose`, `antigravity`, `antigravity-cli`, `zed`, `kiro`, `trae`, `warp`, `cline`, `gjc`, `grok`, `jcode`, `micode`, `commandcode`, `junie`, `zcode`, `synthetic`.

> **Breaking change (v4.0.0):** 클라이언트별 boolean 플래그(`--opencode`, `--claude`, `--codex` 등)는 제거되었으며 이제 오류를 발생시킵니다. 대신 정식 `--client`/`-c` 플래그를 사용하세요 — 예: `tokscale --client opencode,claude`.

### 날짜 필터링

날짜 필터는 리포트를 생성하는 모든 명령어에서 작동합니다 (`tokscale`, `tokscale models`, `tokscale monthly`, `tokscale graph`):

```bash
# 빠른 날짜 단축키
tokscale --today              # 오늘만
tokscale --yesterday          # 어제만
tokscale --week               # 최근 7일
tokscale --month              # 이번 달

# 사용자 정의 날짜 범위 (포함, 로컬 타임존)
tokscale --since 2024-01-01 --until 2024-12-31

# 연도별 필터
tokscale --year 2024

# 다른 옵션과 조합
tokscale models --week --client claude --json
tokscale monthly --month --benchmark
```

> **참고**: 날짜 필터는 로컬 타임존을 사용합니다. `--since`와 `--until` 모두 해당 날짜를 포함합니다.
> **v2.2.0 참고**: 세션 활성 시간의 일별 버킷도 로컬 타임존을 사용하므로, UTC가 아닌 환경에서는 활성 시간 날짜가 UTC 일 경계가 아니라 로컬 토큰/비용 리포트 날짜와 맞춰질 수 있습니다.

### 가격 조회

모든 모델의 실시간 가격을 조회합니다:

```bash
# 모델 가격 조회
tokscale pricing "claude-3-5-sonnet-20241022"
tokscale pricing "gpt-4o"
tokscale pricing "grok-code"

# 특정 프로바이더 소스 강제 지정
tokscale pricing "grok-code" --provider openrouter
tokscale pricing "claude-3-5-sonnet" --provider litellm

# 사용자 정의 가격 오버라이드 확인
tokscale pricing list-overrides
```

**조회 전략:**

가격 조회는 다단계 해석 전략을 사용합니다:

1. **사용자 정의 가격 오버라이드** - `~/.config/tokscale/custom-pricing.json`의 정확한 사용자 정의 항목
2. **정확한 일치** - LiteLLM/OpenRouter 데이터베이스에서 직접 조회
3. **별칭 해석** - 친숙한 이름 해석 (예: `big-pickle` → `glm-4.7`)
4. **티어 접미사 제거** - 품질 티어 제거 (`gpt-5.2-xhigh` → `gpt-5.2`)
5. **버전 정규화** - 버전 형식 처리 (`claude-3-5-sonnet` ↔ `claude-3.5-sonnet`)
6. **프로바이더 접두사 매칭** - 일반 접두사 시도 (`anthropic/`, `openai/` 등)
7. **Cursor 모델 가격** - LiteLLM/OpenRouter에 아직 없는 모델의 하드코딩 가격 (예: `gpt-5.3-codex`)
8. **퍼지 매칭** - 부분 모델 이름에 대한 단어 경계 매칭

### 사용자 정의 가격 오버라이드

업스트림 가격 데이터베이스가 아직 정확히 다루지 못하는 모델 ID의 가격을 오버라이드하려면 Tokscale의 설정 디렉터리(기본값은 macOS/Linux의 `~/.config/tokscale/custom-pricing.json`; `TOKSCALE_CONFIG_DIR`가 설정된 경우 동일한 디렉터리로 해석됨)에 `custom-pricing.json`을 생성하세요.

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

오버라이드 가격은 대부분의 API 프로바이더가 가격을 공개하는 방식과 같이 백만 토큰당 달러 단위로 입력하며, Tokscale은 내부적으로 토큰당 요율로 변환합니다. `input_cost_per_million_tokens` 또는 `output_cost_per_million_tokens` 중 적어도 하나는 존재하고 양수여야 하며, 캐시 읽기/캐시 생성 필드는 선택 사항입니다. 복사/붙여넣기 호환성을 위해 `input_cost_per_token`, `output_cost_per_token`, `cache_read_input_token_cost` 같은 LiteLLM 스타일의 토큰당 필드명도 허용되지만, 백만 토큰당 이름이 권장되는 사용자용 형식입니다. 티어나 캐시 가격을 생략하려면 해당 필드를 비워 두세요. 음수이거나 유한하지 않은 값은 잘못된 것으로 처리되어 오타가 회계를 조용히 바꾸지 않도록 해당 모델 항목 전체를 건너뜁니다. 선택적 `source` 및 `notes` 필드는 Tokscale이 무시하므로 사용자 자신의 기록용으로 사용할 수 있습니다.

오버라이드는 정확 일치 전용이며 대소문자를 구분하지 않습니다. Tokscale은 원본 모델 ID를 먼저 확인하고, 그다음 기존 합성 `/models/` 정규화를 확인한 뒤, 일치하는 오버라이드가 없으면 LiteLLM, OpenRouter, Cursor 가격, 퍼지 매칭으로 넘어갑니다. 원본 정확 일치가 정규화된 정확 일치보다 우선하므로, `accounts/fireworks/routers/kimi-k2p6-turbo`는 특정 게이트웨이 모델을 오버라이드할 수 있고 `kimi-k2p6-turbo`는 정규화된 `/models/` 경로를 커버할 수 있습니다. 오버라이드는 시작 시 한 번 로드되므로 파일을 편집한 후에는 명령을 다시 실행하세요. 업스트림 LiteLLM 가격 업데이트를 기다리는 동안 잘못된 모델 가격 버그를 로컬에서 수정하는 권장 방법입니다.

**프로바이더 우선순위:**

여러 일치 항목이 있을 때 원본 모델 제작사가 리셀러보다 우선됩니다:

| 우선 (원본) | 후순위 (리셀러) |
|---------------------|-------------------------|
| `xai/` (Grok) | `azure_ai/` |
| `anthropic/` (Claude) | `bedrock/` |
| `openai/` (GPT) | `vertex_ai/` |
| `google/` (Gemini) | `together_ai/` |
| `meta-llama/` | `fireworks_ai/` |

예시: `grok-code`는 `azure_ai/grok-code-fast-1` ($3.50/$17.50) 대신 `xai/grok-code-fast-1` ($0.20/$1.50)와 일치합니다.

### 소셜 플랫폼 명령어

```bash
# Tokscale 로그인 (GitHub 인증을 위해 브라우저 열기)
tokscale login

# 브라우저 인증 없이 기존 Tokscale API 토큰 저장
tokscale login --token tt_xxx

# 로그인한 사용자 확인
tokscale whoami

# 저장된 API 토큰을 QR 코드로 표시 (다른 기기로 공유할 때 유용)
# {"token":"tt_xxx","username":"..."}를 인코딩 — 아무 QR 리더로 스캔
tokscale qr

# 사용량 데이터를 리더보드에 제출
tokscale submit

# 자격 증명을 기록하지 않고 CI/헤드리스 환경에서 제출
# 우선순위: TOKSCALE_API_TOKEN 환경 변수 > 저장된 자격 증명 파일 (~/.config/tokscale/credentials.json).
# 환경 변수가 설정되면 해당 실행에서는 저장된 파일이 무시됩니다.
TOKSCALE_API_TOKEN=tt_xxx tokscale submit

# 토큰 폐기: 리더보드 사이트의 Settings > API Tokens
# (https://tokscale.ai/settings)를 방문해 해당 토큰 행의 "Revoke"를 클릭.
# 폐기는 즉시 적용됩니다 — 이후 해당 토큰을 사용한 요청은
# HTTP 401 "Invalid API token"을 받습니다.

# 필터와 함께 제출
tokscale submit --client opencode,claude --since 2024-01-01

# 제출될 내용 미리보기 (드라이 런)
tokscale submit --dry-run

# 로그아웃
tokscale logout
```

<img alt="CLI Submit" src="./.github/assets/cli-submit.png" />

### Autosubmit

Autosubmit은 일반적인 `tokscale submit` 흐름을 운영체제 스케줄러에 등록합니다. 터미널에서 수동으로 실행하지 않아도 공개 프로필을 최신 상태로 유지할 수 있어 유용합니다.

```bash
# 주기적 제출을 활성화합니다. macOS에서는 launchd, Linux에서는 가능한 경우 systemd 사용자 타이머
# (폴백으로 cron), Windows에서는 Windows 작업 스케줄러를 사용합니다.
tokscale autosubmit enable --interval 24h

# submit에 전달하는 것과 동일한 클라이언트/날짜 필터를 그대로 지정할 수 있습니다.
tokscale autosubmit enable --interval 2h --client opencode,claude --week

# 저장된 설정과 마지막 실행/오류를 표시합니다.
tokscale autosubmit status
tokscale autosubmit status --json

# 저장된 간격이 지나지 않았더라도 지금 한 번 실행합니다.
tokscale autosubmit run --force

# Autosubmit을 비활성화하고 스케줄러 항목을 제거합니다.
tokscale autosubmit disable
```

예약된 실행은 비대화형입니다. GitHub 인증이나 스타 확인을 묻지 않습니다. `tokscale login --token tt_xxx`를 한 번 실행하거나 스케줄러 환경에서 `TOKSCALE_API_TOKEN`을 설정하세요. Tokscale은 스케줄러 상태를 `settings.json`에 기록하고, 로그를 `~/.config/tokscale/autosubmit/` 아래에 기록하며, 잠금 파일을 사용해 스케줄러 틱이 겹쳐도 두 번 제출하지 않도록 합니다.

### Cursor IDE 명령어

Cursor IDE 지원은 Cursor의 웹 API 내보내기를 사용하며, Tokscale이 `~/.config/tokscale/cursor-cache/usage*.csv`에 캐싱합니다. Tokscale은 `~/.cursor` 아래의 로컬 Cursor Agent CLI 상태를 파싱하지 않습니다.

설정:

1. 브라우저에서 https://www.cursor.com/settings 를 열고 로그인하세요.
2. `WorkosCursorSessionToken` 쿠키 값을 복사하세요:
   - Network 탭: `cursor.com/api/*`로 아무 요청이나 보낸 뒤, `Cookie` 요청 헤더에서 `WorkosCursorSessionToken=` 뒤의 값을 복사합니다.
   - Application 탭: Cookies → `https://www.cursor.com`을 열고 `WorkosCursorSessionToken` 값을 복사합니다.
3. `tokscale cursor login --name work`를 실행하고 토큰을 붙여 넣으세요.
4. `tokscale cursor sync --json`을 실행해 `~/.config/tokscale/cursor-cache/usage.csv`를 채우세요.
5. `tokscale --client cursor` 또는 아무 리포트 명령을 실행하세요.

세션 토큰은 비밀번호처럼 취급하세요. 토큰은 `~/.config/tokscale/cursor-credentials.json`에 로컬로 저장됩니다.

```bash
# Cursor 로그인 (브라우저에서 세션 토큰 필요)
# --name은 선택이며, 나중에 계정을 구분하는 데만 도움이 됩니다
tokscale cursor login --name work

# Cursor 인증 상태 및 세션 유효성 확인
tokscale cursor status

# 저장된 Cursor 계정 목록
tokscale cursor accounts

# 캐시된 Cursor 사용량 수동 새로고침
tokscale cursor sync --json

# 활성 계정 전환 (cursor-cache/usage.csv에 동기화되는 계정 제어)
tokscale cursor switch work

# 특정 계정 로그아웃 (기록은 보관, 합산에서는 제외)
tokscale cursor logout --name work

# 로그아웃 + 해당 계정 캐시 삭제
tokscale cursor logout --name work --purge-cache

# 모든 Cursor 계정 로그아웃 (기록은 보관, 합산에서는 제외)
tokscale cursor logout --all

# 모든 계정 로그아웃 + 캐시 삭제
tokscale cursor logout --all --purge-cache
```

기본적으로 Tokscale은 `cursor-cache/usage*.csv`를 읽어 저장된 모든 Cursor 계정의 사용량을 합산합니다. 활성 계정은 `usage.csv`에 동기화되고, 추가 계정은 `usage.<account>.csv`에 동기화됩니다.

로그아웃 시 Tokscale은 캐시된 사용량을 `cursor-cache/archive/`로 옮겨 더 이상 합산되지 않도록 합니다. 캐시된 사용량을 대신 삭제하려면 `--purge-cache`를 사용하세요.

### Antigravity 명령어

Antigravity 동기화는 현재 macOS와 Linux에서만 지원됩니다. Antigravity가 활성화된 에디터가 실행 중이고 로컬 언어 서버를 사용할 수 있을 때만 동작하며, tokscale은 해당 로컬 언어 서버에서 사용량을 읽어 정규화된 아티팩트를 로컬에 캐시합니다.

```bash
# 실행 중인 Antigravity 언어 서버를 tokscale이 인식하는지 확인
tokscale antigravity status

# 로컬 Antigravity 언어 서버에서 사용량을 tokscale 캐시로 동기화
tokscale antigravity sync

# 캐시된 Antigravity 아티팩트 삭제
tokscale antigravity purge-cache
```

**캐시 위치**: `~/.config/tokscale/antigravity-cache/`

**동작 방식**: `tokscale antigravity sync`는 로컬 Antigravity 세션 후보를 검색하고, 로컬 언어 서버 RPC에서 확정된 사용량 데이터를 가져와, tokscale-core가 나중에 파싱할 수 있도록 정규화된 JSONL 아티팩트로 저장합니다. 가장 최신의 Antigravity 데이터를 반영하려면 리포트 실행 전에 sync를 먼저 실행하세요.

### Trae 명령어

Trae([ByteDance의 AI IDE](https://www.trae.ai/))는 두 국제판 제품군을 제공합니다. 사용량 데이터는 계정 단위로 공유되므로 tokscale은 하나의 `trae` 클라이언트로 표시합니다:

- **`--variant ide`** — Trae IDE (국제판)의 자격 증명 사용
- **`--variant solo`** — Trae Solo (국제판)의 자격 증명 사용

`tokscale trae sync`는 공식 `query_user_usage_group_by_session` API를 호출하고 원본 JSON을 로컬 캐시에 저장합니다. `--variant solo` / `--variant ide`는 `login`/`logout`에서 자격 증명 출처를 선택할 때만 사용하며, sync는 저장된 Trae 토큰으로 단일 `trae` 리포트 클라이언트를 갱신합니다.

```bash
# 로그인 (Trae 데스크톱 클라이언트에서 자격 증명 자동 감지)
tokscale trae login

# 수동 JWT 입력 (storage.json을 자동 감지할 수 없는 환경용)
tokscale trae login --manual --variant solo

# 어떤 변형에 자격 증명이 캐시되어 있는지 확인
tokscale trae status

# 최근 30일 사용량을 동기화
tokscale trae sync --since 30

# 특정 변형의 캐시된 자격 증명 삭제
tokscale trae logout --variant solo
```

**캐시 위치**: `~/.config/tokscale/trae-cache/`

**동작 방식**: tokscale은 데스크톱 클라이언트의 `iCubeAuthInfo://*` blob(`globalStorage/storage.json`)을 복호화해 JWT를 얻거나, `--manual`로 붙여 넣은 JWT를 사용합니다. 이후 `POST /trae/api/v1/pay/query_user_usage_group_by_session`을 페이지 단위로 호출하고 원본 JSON을 저장합니다. 최신 Trae 데이터를 반영하려면 리포트 실행 전에 sync를 먼저 실행하세요.

> **가격에 대한 참고**: Trae 비용 수치는 **벤더가 보고한 값**입니다 — tokscale은 토큰 수로부터 tokscale의 가격 엔진을 통해 비용을 재계산하는 대신 Trae 자체 API가 반환한 `dollar_float` 값을 그대로 표시합니다. 따라서 수치는 동일한 사용량에 대해 tokscale이 계산했을 값이 아니라 `trae.ai/account-setting#usage`에서 보이는 값과 일치합니다.

> **중국판**: 중국판(`trae.com.cn`)은 의도적으로 지원하지 않습니다. CN 백엔드는 세션 단위 사용량 조회 API를 공개하지 않습니다. 공식 엔드포인트가 제공되면 지원을 추가할 예정입니다.

### Warp/Oz 명령어

Warp/Oz는 로컬 토큰 트랜스크립트를 제공하지 않습니다. Tokscale은 Warp의 GraphQL API가 반환하는 집계 요청 수와 비용 카운터만 동기화하며, 이를 토큰 버킷이 0인 `warp` / `aggregate-requests` 행으로 표시합니다.

```bash
# 인증된 Warp 요청에서 복사한 bearer 토큰 또는 Cookie 헤더 저장
tokscale warp login

# 자격 증명/캐시 상태 및 진단 확인
tokscale warp status

# 집계된 요청 수와 비용을 tokscale 로컬 캐시에 동기화
tokscale warp sync

# 저장된 자격 증명 삭제; --purge-cache를 추가하면 동기화된 사용량도 삭제
tokscale warp logout --purge-cache
```

**캐시 위치**: `~/.config/tokscale/warp-cache/usage.json`

**동작 방식**: `tokscale warp sync`는 Warp의 인증된 GraphQL API를 호출하여 계정 및 워크스페이스 집계 카운터를 가져옵니다. Tokscale은 요청 수를 메시지 카운트로, 벤더가 보고한 비용을 그대로 보존하지만, 요청 수를 합성 토큰으로 변환하지는 않습니다. Warp는 공개 리더보드가 토큰 기반 사용량만 수용하므로 기본 `submit` 데이터에서 제외됩니다.

### 작업 기반 리포트

`report` 명령어는 작업 기반 사용량 분석을 생성합니다. LLM을 사용해 각 세션을 짧은 제목과 카테고리로 요약한 뒤, 관련 세션들을 상위 수준의 작업 클러스터로 묶어 토큰이 어디에 쓰였는지 한눈에 볼 수 있게 해 줍니다.

```bash
# 기본 리포트 (오늘, 기본 Apple FM 요약기)
tokscale report

# 최근 7일
tokscale report --week

# Claude Code를 요약 백엔드로 사용
tokscale report --week --summarizer claude

# Codex, Gemini 또는 Kiro 사용
tokscale report --summarizer codex
tokscale report --summarizer gemini
tokscale report --summarizer kiro

# LLM 요약 건너뛰기 (원본 데이터만 표시)
tokscale report --no-summarize

# 처음부터 다시 요약 (범위 내 캐시된 요약 초기화)
tokscale report --week --rebuild

# JSON으로 출력
tokscale report --week --json

# 워크스페이스 또는 클라이언트로 필터
tokscale report --workspace my-project --client opencode
```

**요약 백엔드:**

| 백엔드 | 명령어 | 비고 |
|---------|---------|-------|
| `apple-fm` | (기본값) | 네이티브 Rust FFI를 통한 온디바이스 Apple Foundation Models (Python 불필요). 사전 빌드된 Apple Silicon(macOS arm64) 바이너리에 기본 포함되어 있으며, Apple Intelligence가 켜진 macOS 26 이상에서 동작합니다. 그 외 환경(Intel Mac, 이전 macOS, Linux, Windows)에서는 내장 Rust 휴리스틱으로 투명하게 폴백하므로 기본값은 모든 플랫폼에서 동작합니다. |
| `claude` | `claude -p` | Claude Code CLI가 설치되어 인증되어 있어야 함. |
| `codex` | `codex --quiet` | Codex CLI가 설치되어 인증되어 있어야 함. |
| `gemini` | `gemini -p` | Gemini CLI가 설치되어 인증되어 있어야 함. |
| `kiro` | `kiro --non-interactive` | Kiro CLI가 설치되어 인증되어 있어야 함. |

**동작 방식:**

1. 세션을 스캔하여 로컬 SQLite 위키 데이터베이스(`wiki.db`, 플랫폼 설정 디렉터리 — Linux: `~/.config/tokscale/`, macOS: `~/Library/Application Support/tokscale/`)에 삽입합니다
2. 요약되지 않은 세션을 선택한 LLM 백엔드에 배치 단위로 보내면, 각 세션에 대해 제목, 카테고리, 설명, 복잡도를 반환합니다
3. 두 번째 LLM 패스에서 제목이 붙은 모든 세션을 3~8개의 상위 수준 작업 클러스터로 묶습니다 (예: "Kiro Auth", "Tokscale Report", "System Config")
4. 결과는 위키 DB에 캐시되며, 이후 실행 시 이미 요약된 세션은 건너뜁니다

요약은 기본적으로 활성화되어 있으며 기본 백엔드는 `apple-fm`(네이티브 Rust를 통한 Apple Foundation Models 온디바이스 추론, Python 불필요)입니다. `--no-summarize`로 요약을 끌 수 있습니다.

**예시 출력:**

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

### 구독 사용량

Tokscale은 여러 AI 프로바이더에 걸친 실시간 구독 할당량을 가져와 표시할 수 있습니다. 이를 통해 플랜을 얼마나 사용했는지와 한도가 언제 초기화되는지 확인할 수 있습니다.

```bash
# 감지된 모든 프로바이더의 구독 사용량 표시
tokscale usage

# JSON으로 출력 (스크립팅용)
tokscale usage --json

# 가벼운 터미널 출력 (TUI 없음)
tokscale usage --light
```

TUI에서는 **Usage** 탭으로 이동해 구독 데이터를 확인하세요. `[Refresh]`로 구독 할당량을 새로고침할 수 있습니다. 키보드 새로고침 단축키 `r`도 동일한 새로고침 경로를 사용합니다.

> **참고**: 구독 할당량과 잔액은 **벤더가 보고한 값**입니다 — tokscale은 각 프로바이더의 자체 할당량 엔드포인트를 호출하고 그 응답을 그대로 표시합니다. 표시되는 수치는 프로바이더가 보고하는 값(공식 대시보드에 나타나는 값과 동일)이며, tokscale 자체 사용량 추적과 독립적으로 검증되지 않습니다.

#### 지원 프로바이더

| 프로바이더 | 인증 방식 | 지표 | 설정 |
|----------|-------------|---------|-------|
| **Claude** | OAuth (자격 증명 파일 또는 macOS Keychain) | 세션(5시간), 주간, Opus 할당량 | `claude`를 실행해 로그인 |
| **Codex** (OpenAI) | OAuth (`~/.config/codex/auth.json`, `~/.codex/auth.json`, 또는 저장된 Tokscale 계정) | 세션, 주간 할당량 | TUI Usage 탭에서 `[Add Codex]`를 사용하거나, `codex`를 실행해 로그인하거나, `tokscale codex import --name work`로 기존 인증을 가져오기 |
| **Z.ai** | API 키 (환경 변수) | 토큰 한도, 웹 검색 | `ZAI_API_KEY` 또는 `GLM_API_KEY` 설정 |
| **Amp** | API 키 (`~/.local/share/amp/secrets.json`) | 무료 티어 잔액, 크레딧 | `amp`를 실행해 로그인 |
| **GitHub Copilot** | GitHub 토큰 (keychain 또는 `~/.config/gh/hosts.yml`) | 프리미엄 상호작용, 채팅 할당량 | `gh auth login` 실행 |
| **Grok Build** | OAuth (`~/.grok/auth.json`) | 크레딧, 구독 플랜 | `grok login` 실행 |
| **Kimi** | OAuth (`~/.kimi/credentials/kimi-code.json`) | 세션, 주간 할당량 | `kimi`를 실행해 로그인 |
| **MiniMax** | API 키 (환경 변수) | 모델별 프롬프트 할당량 | `MINIMAX_API_KEY` 또는 `MINIMAX_API_TOKEN` 설정 |
| **MiniMax Token Plan** | API 키 (환경 변수) | 구간 + 주간 잔여 비율 할당량 (지역별: CN minimaxi.com + Global minimax.io) | `MINIMAX_TOKEN_PLAN_CN_KEY` 및/또는 `MINIMAX_TOKEN_PLAN_GLOBAL_KEY` 설정 |
| **Sakana** (Fugu) | 세션 쿠키 (환경 변수 또는 파일) — 빌링 콘솔 HTML 스크레이프, 공개 API 없음 | 5시간, 주간 할당량 창 (플랜 티어 + 월 가격은 메타데이터) | `SAKANA_SESSION_COOKIE` 설정 ([docs/providers/sakana.md](docs/providers/sakana.md) 참조) |

프로바이더는 자동 감지됩니다 — 유효한 자격 증명이 있는 프로바이더만 표시됩니다. 프로바이더가 보이지 않으면 로그인했는지 또는 필요한 환경 변수를 설정했는지 확인하세요.

#### Codex 다중 계정 사용량

Tokscale은 구독 사용량 표시를 위해 여러 Codex OAuth 계정을 저장할 수 있습니다. TUI Usage 탭은 저장된 계정들을 하나의 **Codex** 섹션 아래에 묶습니다. 활성 계정은 `*`로 표시되고, 비활성 계정은 `[Use]`로 선택할 수 있으며, 계정 삭제는 `[Remove]` 후 `[Confirm]`으로 진행합니다.

TUI를 벗어나지 않고 계정을 추가하려면 Usage 탭에서 `[Add Codex]`를 클릭하세요. Tokscale은 임시 `CODEX_HOME`으로 `codex login`을 시작하고, 로그인 출력을 Usage 탭에 표시한 뒤, 결과 인증을 Tokscale의 저장 계정 스토어로 가져오고, 사용량을 새로고침합니다. 이렇게 하면 로그인이 격리되며 현재 Codex 인증을 전환하지 않습니다. Tokscale이 실제 Codex 인증 파일에 특정 계정을 쓰게 하려면 저장된 계정에서 `[Use]`를 클릭하세요.

스크립트 기반 또는 수동 계정 관리를 위한 CLI 명령도 계속 제공됩니다:

```bash
# 현재 Codex 인증을 이름 있는 Tokscale 계정으로 저장
tokscale codex import --name work

# 저장된 Codex 계정 목록
tokscale codex accounts
tokscale codex accounts --json

# 활성 Codex 계정 전환 및 Codex auth.json 기록
tokscale codex switch work

# 저장된 Codex 계정 추적 중지 (Tokscale 스토어에서만 제거 —
# codex CLI 자체의 auth.json/로그인은 절대 건드리지 않음)
tokscale codex remove personal

# 활성 또는 이름 있는 계정의 구독 사용량 확인
tokscale codex status
tokscale codex status --name personal --json
```

저장된 Codex 계정이 있으면 `tokscale usage --json`은 각 Codex 항목에 대한 구조화된 계정 메타데이터를 포함하며 TUI는 해당 항목들을 하나의 Codex 그룹 아래에 표시합니다. 저장된 계정이 없으면 Tokscale은 현재 Codex 인증 탐색 경로(`CODEX_HOME/auth.json`, `~/.config/codex/auth.json`, `~/.codex/auth.json`, 그리고 macOS Keychain)로 폴백합니다.

#### 예시 출력

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

### 예시 출력 (`--light` 버전)

<img alt="CLI Light" src="./.github/assets/cli-light.png" />

### 설정

Tokscale은 설정을 `~/.config/tokscale/settings.json`에 저장합니다:

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

| 설정 | 타입 | 기본값 | 설명 |
|---------|------|---------|-------------|
| `colorPalette` | string | `"blue"` | TUI 색상 테마 (green, halloween, teal, blue, pink, purple, orange, monochrome, ylgnbu, graphite, lagoon, dusk) |
| `includeUnusedModels` | boolean | `false` | 리포트에서 제로 토큰 모델 표시 |
| `autoRefreshEnabled` | boolean | `false` | TUI 자동 새로고침 활성화 |
| `autoRefreshMs` | number | `60000` | 자동 새로고침 간격 (30000-3600000ms) |
| `nativeTimeoutMs` | number | `300000` | 네이티브 서브프로세스 처리 최대 시간 (5000-3600000ms) |
| `defaultClients` | string[] | `[]` | `--client/-c` 플래그를 전달하지 않을 때 적용되는 기본 클라이언트 필터. `--client`와 동일한 ID를 받습니다 (예: `["opencode", "claude", "synthetic"]`). 알 수 없는 ID는 자동으로 무시됩니다. CLI 플래그가 있으면 이 목록은 완전히 무시됩니다 — 병합되지 않습니다. |
| `light.writeCache` | boolean | `false` | `true`이면 `tokscale --light`가 렌더링 직후 TUI 캐시를 원자적으로 덮어씁니다. CLI 플래그 `--write-cache` / `--no-write-cache`가 실행별로 우선합니다. |
| `minutelyTabEnabled` | boolean | `false` | TUI에 분 단위 Minutely 탭을 표시하고 데이터 로딩 중에 분 단위 집계를 수행합니다. 대부분의 사용자에게 분 단위 세분화는 틈새/진단 뷰이며, 대규모 데이터셋에서는 분 단위 버케팅에 무시할 수 없는 비용이 들기 때문에 기본적으로 비활성화되어 있습니다. |
| `scanner.extraScanPaths` | object | `{}` | Tokscale의 기본 home-root 위치 밖에 있는 세션을 위한 클라이언트별 추가 스캔 루트 |

`scanner.extraScanPaths`는 프로젝트 단위 `.codex` 디렉터리나 가져온 Gemini/OpenClaw 히스토리 같은 영구적인 추가 루트에 사용하세요. Tokscale은 `$HERMES_HOME/profiles/*/state.db` 아래의 Hermes 프로필 데이터베이스를 자동으로 발견합니다(`HERMES_HOME`이 없으면 `~/.hermes/profiles/*/state.db`). 비표준 Hermes 프로필 위치에만 `scanner.extraScanPaths.hermes`를 사용하세요. Hermes 항목은 `state.db`를 포함하는 프로필 디렉터리를 가리키거나 `state.db` 파일을 직접 가리킬 수 있습니다. Tokscale은 매 실행마다 이 경로들을 기본 스캔 루트와 병합하고, 겹치는 루트는 정규 경로(canonical path) 기준으로 중복 제거합니다.

`defaultClients`로 개인 기본값을 고정할 수 있습니다 — 예를 들어 OpenCode와 Claude만 사용한다면 `["opencode", "claude"]`로 설정하면, `tokscale`(플래그 없이)은 모든 리포트를 자동으로 해당 클라이언트로 범위를 좁힙니다. 단일 실행에 대해 재정의하려면 명령줄에서 `--client`를 전달하세요.

#### Minutely 탭 활성화

Minutely 탭은 토큰 사용량을 분 단위로 표시하며, 버스트 패턴 진단, 단일 세션 디버깅, `autoRefreshEnabled`와 함께 거의 실시간 모니터링에 가장 유용합니다. 분 단위 집계는 데이터 로딩 중 모든 파싱된 메시지를 처리하므로 대부분의 사용자에게는 불필요한 RAM과 CPU 비용이 발생합니다. 그래서 기본적으로 숨겨져 있습니다.

활성화하려면 `~/.config/tokscale/settings.json`에서 `minutelyTabEnabled`를 `true`로 설정하세요:

```json
{
  "minutelyTabEnabled": true
}
```

재시작 후 탭 스트립의 Hourly와 Stats 사이에 Minutely 탭이 나타나며, Tab / BackTab / Left / Right 내비게이션이 이를 순환합니다. 플래그를 다시 `false`로 설정하면 탭이 숨겨지고 집계도 다시 건너뜁니다.

#### 캐시 디렉터리 레이아웃

재생성 가능한 CLI/TUI/가격/Wrapped 캐시는 `~/.config/tokscale/cache/` 아래에 저장됩니다 (`TOKSCALE_CONFIG_DIR`를 설정한 경우 `${TOKSCALE_CONFIG_DIR}/cache/`). 통합 동기화 아티팩트는 `~/.config/tokscale/antigravity-cache/` 및 `~/.config/tokscale/trae-cache/` 같은 클라이언트별 캐시 루트에 저장됩니다:

- `tui-data-cache.json` — TUI 시작 캐시
- `source-message-cache-v2/` + `source-message-cache.lock` — 샤딩된 소스 메시지 캐시와 락 파일
- `pricing-litellm.json` / `pricing-openrouter.json` — 가격 캐시
- `opencode-migration.json` — OpenCode 마이그레이션 기록
- `fonts/`, `images/` — Wrapped 에셋 캐시

이 디렉터리는 삭제해도 안전합니다. 필요할 때 Tokscale이 다시 생성하고 채웁니다.

### 환경 변수

환경 변수는 설정 파일 값을 오버라이드합니다. CI/CD 또는 일회성 사용:

| 변수 | 기본값 | 설명 |
|----------|---------|-------------|
| `TOKSCALE_NATIVE_TIMEOUT_MS` | `300000` (5분) | `nativeTimeoutMs` 설정 오버라이드 |
| `TOKSCALE_API_TOKEN` | unset | 비대화형 `submit` 및 `delete-submitted-data` 실행을 위한 Tokscale 개인 API 토큰. Settings > API Tokens에서 생성하거나 `tokscale login --token tt_xxx`로 로컬에 저장하세요. |
| `TOKSCALE_EXTRA_DIRS` | unset | 일회성 추가 세션 루트, `client:/abs/path,client:/abs/path` 형식 |
| `TOKSCALE_CONFIG_DIR` | unset | 설정 디렉토리 루트(`settings.json`, `star-cache.json`, `cache/`, `antigravity-cache/`, `trae-cache/` 위치)를 오버라이드합니다. 절대 경로 권장; 상대 경로는 프로세스 CWD 기준으로 해석됩니다. CI 샌드박스나 비기본 위치를 고정할 때 유용합니다. 설정되면 tokscale은 macOS 레거시 경로(`~/Library/Application Support/tokscale/`)로 폴백하지 않습니다. |
| `TOKSCALE_FM_DEBUG` | unset | 설정되면 Apple Foundation Models 진단 정보(macOS 버전 게이트, dlopen dylib 경로, 로드/심볼 오류)를 stderr로 출력하여 온디바이스 apple-fm이 동작했는지 또는 동작하지 않았는지 이유를 설명합니다. |

```bash
# 예시: 매우 큰 데이터셋에 대한 타임아웃 증가
TOKSCALE_NATIVE_TIMEOUT_MS=600000 tokscale graph --output data.json

# 예시: 일회성 추가 스캔 루트
TOKSCALE_EXTRA_DIRS='codex:/Users/me/workspace/project-a/.codex/sessions,gemini:/Users/me/imports/imac/gemini/tmp' tokscale

# 예시: 대화형 브라우저 로그인 없이 CI에서 제출
TOKSCALE_API_TOKEN=tt_xxx tokscale submit
```

> **참고**: 영구적인 추가 루트는 `~/.config/tokscale/settings.json`의 `scanner.extraScanPaths`를 권장합니다. `TOKSCALE_EXTRA_DIRS`는 일회성 오버라이드나 CI/CD에 가장 적합합니다.

### Headless 모드

Tokscale은 자동화, CI/CD 파이프라인 및 배치 처리를 위한 **Codex CLI headless 출력**의 토큰 사용량을 집계할 수 있습니다.

**Headless 모드란?**

Codex CLI를 JSON 출력 플래그와 함께 실행할 때(예: \`codex exec --json\`), 일반 세션 디렉토리에 저장하는 대신 사용량 데이터를 stdout으로 출력합니다. Headless 모드를 사용하면 이러한 사용량을 캡처하고 추적할 수 있습니다.

**저장 위치:** \`~/.config/tokscale/headless/\`

macOS에서는 \`TOKSCALE_HEADLESS_DIR\`이 설정되지 않은 경우 Tokscale이 \`~/Library/Application Support/tokscale/headless/\`도 스캔합니다.

Tokscale은 다음 디렉토리 구조를 자동으로 스캔합니다:
\`\`\`
~/.config/tokscale/headless/
└── codex/       # Codex CLI JSONL 출력
\`\`\`

**환경 변수:** \`TOKSCALE_HEADLESS_DIR\`을 설정하여 headless 로그 디렉토리를 커스터마이징할 수 있습니다:
\`\`\`bash
export TOKSCALE_HEADLESS_DIR="$HOME/my-custom-logs"
\`\`\`

**권장 (자동 캡처):**

| 도구 | 명령어 예시 |
|------|-------------|
| **Codex CLI** | \`tokscale headless codex exec -m gpt-5 "implement feature"\` |

**수동 리다이렉트 (선택사항):**

| 도구 | 명령어 예시 |
|------|-------------|
| **Codex CLI** | \`codex exec --json "implement feature" > ~/.config/tokscale/headless/codex/ci-run.jsonl\` |

**진단:**

\`\`\`bash
# 스캔 위치 및 headless 카운트 표시
tokscale sources
tokscale sources --json
\`\`\`

**CI/CD 통합 예시:**

\`\`\`bash
# GitHub Actions 워크플로우에서
- name: Run AI automation
  run: |
    mkdir -p ~/.config/tokscale/headless/codex
    codex exec --json "review code changes" \\
      > ~/.config/tokscale/headless/codex/pr-\${{ github.event.pull_request.number }}.jsonl

# 나중에 사용량 추적
- name: Report token usage
  run: tokscale --json
\`\`\`

> **참고**: Headless 캡처는 Codex CLI만 지원됩니다. Codex를 직접 실행하는 경우 위와 같이 stdout을 headless 디렉토리로 리다이렉트해야 합니다.

## 프론트엔드 시각화

프론트엔드는 GitHub 스타일의 기여 그래프 시각화를 제공합니다:

### 기능

- **2D 뷰**: 클래식 GitHub 기여 캘린더
- **3D 뷰**: 토큰 사용량에 따른 높이의 아이소메트릭 3D 기여 그래프
- **다양한 색상 팔레트**: GitHub, GitLab, Halloween, Winter 등
- **3가지 테마 토글**: Light / Dark / System (OS 설정 따름)
- **GitHub Primer 디자인**: GitHub의 공식 색상 시스템 사용
- **인터랙티브 툴팁**: 호버 시 상세 일별 분석 표시
- **일별 분석 패널**: 클릭하여 소스별, 모델별 세부사항 확인
- **연도 필터링**: 연도 간 탐색
- **소스 필터링**: 플랫폼별 필터 (OpenCode, Claude, Codex, Copilot, Cursor, Gemini, Amp, Codebuff, Droid, OpenClaw, Hermes Agent, Pi, Kimi, Qwen, Roo Code, Kilo, Mux, Kilo CLI, Crush, Goose, Antigravity, Antigravity CLI, Zed, Kiro, Trae, Warp, Cline, Gajae-Code, Grok Build, Jcode, MiMo Code, Command Code, Junie, ZCode, Synthetic)
- **통계 패널**: 총 비용, 토큰, 활동 일수, 연속 기록
- **FOUC 방지**: React 하이드레이션 전 테마 적용 (깜빡임 없음)

### 프론트엔드 실행

```bash
cd packages/frontend
bun install
bun run dev
```

[http://localhost:3000](http://localhost:3000)을 열어 소셜 플랫폼에 접근하세요.

## 소셜 플랫폼

Tokscale은 사용량 데이터를 공유하고 다른 개발자와 경쟁할 수 있는 소셜 플랫폼을 포함합니다.

### 기능

- **리더보드** - 모든 플랫폼에서 가장 많은 토큰을 사용하는 사람 확인
- **사용자 프로필** - 기여 그래프와 통계가 있는 공개 프로필
- **기간 필터링** - 전체 기간, 이번 달, 이번 주 통계 조회
- **GitHub 통합** - GitHub 계정으로 로그인
- **로컬 뷰어** - 제출하지 않고 비공개로 데이터 조회

### GitHub 프로필 임베드 위젯

GitHub 프로필 README에 Tokscale 공개 통계를 직접 임베드할 수 있습니다:

```md
[![Tokscale Stats](https://tokscale.ai/api/embed/<username>/svg)](https://tokscale.ai/u/<username>)
```

`<username>`을 GitHub 사용자명으로 교체하세요. 쿼리 파라미터가 없으면
기본 `classic` 카드가 렌더링됩니다. 디자인을 커스터마이즈하려면 아래
파라미터를 덧붙이세요.

| 파라미터 | 값 | 효과 |
| --- | --- | --- |
| `template` | `classic` (기본값) · `minimal` · `terminal` · `graph` · `orbit` · `vitals` · `blueprint` · `receipt` | 카드 디자인 |
| `color` | `blue` · `green` · `teal` · `purple` · `pink` · `orange` · `monochrome` · `halloween` · `YlGnBu` | 강조 색상 및 기여 그래프 팔레트 |
| `theme` | `dark` (기본값) · `light` | 라이트 또는 다크 카드 |
| `sort` | `tokens` (기본값) · `cost` | 랭크를 가져올 리더보드 기준 |
| `tokens`, `cost` | `compact` · `full` | 숫자 형식, 독립적으로 설정 — `20.9B` vs `20,941,000,000` |
| `rank` | `plain` (기본값, `#134`) · `percent` (`top 12%`) · `total` (`#134 / 1,174`) | 리더보드 랭크 표시 방식 |
| `graph` | `1`로 기여 그래프 추가 (기본값은 꺼짐) | `classic`, `minimal`, `terminal`, `orbit`, `blueprint`, `receipt`에서 지원 |
| `compact` | `1`로 컴팩트 레이아웃 사용 | `classic` 전용 |

예시:

```md
![](https://tokscale.ai/api/embed/<username>/svg?template=minimal&color=purple&graph=1)
![](https://tokscale.ai/api/embed/<username>/svg?template=orbit&color=pink&rank=percent)
![](https://tokscale.ai/api/embed/<username>/svg?template=terminal&color=green&theme=light)
![](https://tokscale.ai/api/embed/<username>/svg?template=receipt&color=YlGnBu&graph=1)
```

### GitHub 프로필 뱃지

shields.io 스타일의 더 간결한 뱃지를 사용할 수도 있습니다:

```md
![Tokscale Tokens](https://tokscale.ai/api/badge/<username>/svg)
```

- `<username>`을 GitHub 사용자명으로 교체하세요
- 선택적 쿼리 파라미터:
  - `metric=tokens` (기본값), `metric=cost`, 또는 `metric=rank`
  - `style=flat` (기본값) 또는 `style=flat-square`
  - `sort=tokens` (기본값) 또는 `sort=cost` 랭킹 기준 제어
  - `compact=1` 간결한 숫자 표기 사용 (예: `1.2M`, `$3.4K`)
  - `label=<텍스트>` 왼쪽 라벨 커스텀
  - `color=<hex>` 오른쪽 배경색 커스텀 (예: `color=ff5733`)
- 예시:
  - `https://tokscale.ai/api/badge/<username>/svg?metric=cost&compact=1`
  - `https://tokscale.ai/api/badge/<username>/svg?metric=rank&sort=cost&style=flat-square`

### 시작하기

1. **로그인** - `tokscale login`을 실행하여 GitHub로 인증
2. **제출** - `tokscale submit`을 실행하여 사용량 데이터 업로드
3. **조회** - 웹 플랫폼을 방문하여 프로필과 리더보드 확인

### 데이터 검증

제출된 데이터는 레벨 1 검증을 거칩니다:
- 수학적 일관성 (합계 일치, 음수 없음)
- 미래 날짜 없음
- 필수 필드 존재
- 중복 감지

## Wrapped 2025

![Wrapped 2025](.github/assets/hero-wrapped-2025.png)

Spotify Wrapped에서 영감을 받아, AI 코딩 어시스턴트 사용량을 요약한 아름다운 연간 리뷰 이미지를 생성합니다.

| `bunx tokscale@latest wrapped` | `bunx tokscale@latest wrapped --clients` | `bunx tokscale@latest wrapped --agents --disable-pinned` |
|:---:|:---:|:---:|
| ![Wrapped 2025 (Agents + Pin Sisyphus)](.github/assets/wrapped-2025-agents.png) | ![Wrapped 2025 (Clients)](.github/assets/wrapped-2025-clients.png) | ![Wrapped 2025 (Agents + Disable Pinned)](.github/assets/wrapped-2025-agents-disable-pinned.png) |

### 명령어

```bash
# 현재 연도의 Wrapped 이미지 생성
tokscale wrapped

# 특정 연도의 Wrapped 이미지 생성
tokscale wrapped --year 2025
```

### 포함 내용

생성된 이미지에는 다음이 포함됩니다:

- **총 토큰** - 해당 연도의 총 토큰 소비량
- **상위 모델** - 비용 기준 상위 3개 AI 모델
- **상위 클라이언트** - 가장 많이 사용한 3개 플랫폼 (OpenCode, Claude Code, Cursor 등)
- **메시지** - 총 AI 인터랙션 수
- **활동 일수** - 최소 1회 이상 AI 인터랙션이 있었던 일수
- **비용** - LiteLLM 가격 기준 추정 총비용
- **연속 기록** - 가장 긴 연속 활동 일수
- **기여 그래프** - 연간 활동을 보여주는 히트맵

생성된 PNG는 소셜 미디어 공유에 최적화되어 있습니다. 커뮤니티와 함께 코딩 여정을 공유하세요!

## 개발

> **빠른 설정**: 빠르게 시작하려면 위 설치 섹션의 [개발 환경 설정](#개발-환경-설정)을 참조하세요.

### 사전 요구사항

```bash
# Bun (필수)
bun --version

# Rust (네이티브 모듈용)
rustc --version
cargo --version
```

### 실행 방법

[개발 환경 설정](#개발-환경-설정)을 따른 후:

```bash
# 네이티브 모듈 빌드 (선택사항이지만 권장)
bun run build:core

# 개발 모드로 실행 (TUI 실행)
cd packages/cli && bun src/index.ts

# 또는 레거시 CLI 모드 사용
cd packages/cli && bun src/index.ts --light
```

<details>
<summary>고급 개발</summary>

### 프로젝트 스크립트

| 스크립트 | 설명 |
|--------|-------------|
| `bun run cli` | 개발 모드에서 CLI 실행 (Bun으로 TUI) |
| `bun run build:core` | 네이티브 Rust 모듈 빌드 (릴리스) |
| `bun run build:cli` | CLI TypeScript를 dist/로 빌드 |
| `bun run build` | core와 CLI 모두 빌드 |
| `bun run dev:frontend` | 프론트엔드 개발 서버 실행 |

**패키지별 스크립트** (패키지 디렉토리 내에서):
- `packages/cli`: `bun run dev`, `bun run tui`
- `packages/core`: `bun run build:debug`, `bun run test`, `bun run bench`

**참고**: 이 프로젝트는 개발 시 **Bun**을 패키지 매니저로 사용합니다.

### 테스트

```bash
# 네이티브 모듈 테스트 (Rust)
cd packages/core
bun run test:rust      # Cargo 테스트
bun run test           # Node.js 통합 테스트
bun run test:all       # 둘 다
```

### 네이티브 모듈 개발

```bash
cd packages/core

# 디버그 모드로 빌드 (빠른 컴파일)
bun run build:debug

# 릴리스 모드로 빌드 (최적화됨)
bun run build

# Rust 벤치마크 실행
bun run bench
```

### 그래프 명령어 옵션

```bash
# 그래프 데이터를 파일로 내보내기
tokscale graph --output usage-data.json

# 날짜 필터링 (모든 단축키 사용 가능)
tokscale graph --today
tokscale graph --week
tokscale graph --since 2024-01-01 --until 2024-12-31
tokscale graph --year 2024

# 플랫폼별 필터
tokscale graph --client opencode,claude

# 처리 시간 벤치마크 표시
tokscale graph --output data.json --benchmark
```

### 벤치마크 플래그

성능 분석을 위한 처리 시간 표시:

```bash
tokscale --benchmark           # 기본 뷰와 함께 처리 시간 표시
tokscale models --benchmark    # 모델 리포트 벤치마크
tokscale monthly --benchmark   # 월별 리포트 벤치마크
tokscale graph --benchmark     # 그래프 생성 벤치마크
```

### 프론트엔드용 데이터 생성

```bash
# 시각화용 데이터 내보내기
tokscale graph --output packages/frontend/public/my-data.json
```

### 성능

네이티브 Rust 모듈은 상당한 성능 향상을 제공합니다:

| 작업 | TypeScript | Rust 네이티브 | 속도 향상 |
|-----------|------------|-------------|---------|
| 파일 탐색 | ~500ms | ~50ms | **10배** |
| JSON 파싱 | ~800ms | ~100ms | **8배** |
| 집계 | ~200ms | ~25ms | **8배** |
| **총합** | **~1.5초** | **~175ms** | **~8.5배** |

*약 1000개의 세션 파일, 100k 메시지 기준 벤치마크*

#### 메모리 최적화

네이티브 모듈은 다음을 통해 약 45% 메모리 절감도 제공합니다:

- 스트리밍 JSON 파싱 (전체 파일 버퍼링 없음)
- 제로 카피 문자열 처리
- 맵-리듀스를 통한 효율적인 병렬 집계

#### 벤치마크 실행

```bash
# 합성 데이터 생성
cd packages/benchmarks && bun run generate

# Rust 벤치마크 실행
cd packages/core && bun run bench
```

</details>

## 지원 플랫폼

### 네이티브 모듈 대상

| 플랫폼 | 아키텍처 |
|----------|--------------|
| macOS | x86_64 |
| macOS | aarch64 (Apple Silicon) |
| Linux | x86_64 (glibc) |
| Linux | aarch64 (glibc) |
| Linux | x86_64 (musl) |
| Linux | aarch64 (musl) |
| Windows | x86_64 |
| Windows | aarch64 |

Linux에서는 런처가 glibc와 musl을 자동으로 감지합니다 (`process.report`, `/lib/ld-musl-*.so.1`의 musl 동적 로더, 그리고 `ldd`를 통해). 감지가 잘못된 종류를 선택하는 경우 — 예를 들어 최소 컨테이너에서 — `TOKSCALE_LIBC=musl` (또는 `TOKSCALE_LIBC=gnu`)를 설정해 강제할 수 있습니다.

### Windows 지원

Tokscale은 Windows를 완벽하게 지원합니다. TUI와 CLI는 macOS/Linux와 동일하게 작동합니다.

**Windows 설치:**
```powershell
# Bun 설치 (PowerShell)
powershell -c "irm bun.sh/install.ps1 | iex"

# tokscale 실행
bunx tokscale@latest
```

#### Windows에서의 데이터 위치

AI 코딩 도구들은 크로스 플랫폼 위치에 세션 데이터를 저장합니다. 대부분의 도구는 모든 플랫폼에서 동일한 상대 경로를 사용합니다:

| 도구 | Unix 경로 | Windows 경로 | 출처 |
|------|-----------|--------------|--------|
| OpenCode | `~/.local/share/opencode/` | `%USERPROFILE%\.local\share\opencode\` | 크로스 플랫폼 일관성을 위해 [`xdg-basedir`](https://github.com/sindresorhus/xdg-basedir) 사용 ([소스](https://github.com/sst/opencode/blob/main/packages/opencode/src/global/index.ts)) |
| Claude Code | `~/.claude/` | `%USERPROFILE%\.claude\` | 모든 플랫폼에서 동일한 경로 |
| OpenClaw | `~/.openclaw/` (+ 레거시: `.clawdbot`, `.moltbot`, `.moldbot`) | `%USERPROFILE%\.openclaw\` (+ 레거시 경로) | 모든 플랫폼에서 동일한 경로 |
| Codex CLI | `~/.codex/` | `%USERPROFILE%\.codex\` | `CODEX_HOME` 환경변수로 설정 가능 ([소스](https://github.com/openai/codex)) |
| Copilot CLI | `~/.copilot/otel/ ` | `%USERPROFILE%\.copilot\otel\` | OTEL 파일 내보내기 필요; `COPILOT_OTEL_FILE_EXPORTER_PATH`도 자동 수집 |
| Hermes Agent | `~/.hermes/` | `%USERPROFILE%\.hermes\` | `HERMES_HOME` 환경변수로 설정 가능 ([소스](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/session-storage.md)) |
| Gemini CLI | `~/.gemini/` | `%USERPROFILE%\.gemini\` | `GEMINI_CLI_HOME` 환경변수로 설정 가능 |
| Amp | `~/.local/share/amp/` | `%USERPROFILE%\.local\share\amp\` | OpenCode와 동일하게 `xdg-basedir` 사용 |
| Cursor | API 동기화 | API 동기화 | API를 통해 데이터 가져오기, `%USERPROFILE%\.config\tokscale\cursor-cache\`에 캐시 |
| Droid | `~/.factory/` | `%USERPROFILE%\.factory\` | 모든 플랫폼에서 동일한 경로 |
| Pi | `~/.pi/` and `~/.omp/` | `%USERPROFILE%\.pi\` and `%USERPROFILE%\.omp\` | 모든 플랫폼에서 동일한 경로 (Pi 및 [Oh My Pi](https://github.com/can1357/oh-my-pi) 모두 지원) |
| Kimi CLI | `~/.kimi/` | `%USERPROFILE%\.kimi\` | 모든 플랫폼에서 동일한 경로 |
| Kimi Code | `~/.kimi-code/` | `%USERPROFILE%\.kimi-code\` | 모든 플랫폼에서 동일한 경로 |
| Qwen CLI | `~/.qwen/` | `%USERPROFILE%\.qwen\` | 모든 플랫폼에서 동일한 경로 |
| Roo Code | `~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks/` | `%USERPROFILE%\.config\Code\User\globalStorage\rooveterinaryinc.roo-cline\tasks\` | VS Code globalStorage 작업 로그 |
| Kilo | `~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/` | `%USERPROFILE%\.config\Code\User\globalStorage\kilocode.kilo-code\tasks\` | VS Code globalStorage 작업 로그 |
| Cline | Linux: `~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/`; macOS: `~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/`; 서버: `~/.vscode-server/data/User/globalStorage/saoudrizwan.claude-dev/tasks/` | `%APPDATA%\Code\User\globalStorage\saoudrizwan.claude-dev\tasks\` | VS Code globalStorage 작업 로그 |
| Mux | `~/.mux/sessions/` | `%USERPROFILE%\.mux\sessions\` | 모든 플랫폼에서 동일한 경로 |
| Codebuff | `~/.config/manicode/projects/` (+ `manicode-dev`, `manicode-staging`) | `%USERPROFILE%\.config\manicode\projects\` | `CODEBUFF_DATA_DIR` 환경변수로 오버라이드 |
| Kilo CLI | `~/.local/share/kilo/` | `%USERPROFILE%\.local\share\kilo\` | OpenCode와 같이 `xdg-basedir` 사용 |
| Crush | `$XDG_DATA_HOME/crush/` (기본값: `~/.local/share/crush/`) | `%USERPROFILE%\.local\share\crush\` (설정된 경우 `%XDG_DATA_HOME%\crush\`) | 기본 경로를 포함한 XDG 데이터 디렉토리 사용 |
| Goose | `~/.local/share/goose/sessions/` (+ macOS Application Support, 레거시 Block 경로) | `%USERPROFILE%\.local\share\goose\sessions\` | `GOOSE_PATH_ROOT` 환경변수로 설정 가능 |
| Antigravity | `~/.config/tokscale/antigravity-cache/sessions/` | — | `tokscale antigravity sync`는 현재 macOS/Linux에서만 지원 |
| Zed Agent | `~/.local/share/zed/threads/threads.db` | `%LOCALAPPDATA%\Zed\threads\threads.db` | 호스팅된 Zed 모델 사용량 전용; 외부 ACP 에이전트는 포함되지 않음 |
| Kiro | `~/.kiro/sessions/cli/` 및 `~/.local/share/kiro-cli/data.sqlite3` | `%USERPROFILE%\.kiro\sessions\cli\` 및 `%USERPROFILE%\.local\share\kiro-cli\data.sqlite3` | Kiro 세션 파일과 함께 존재하는 경우 Kiro CLI SQLite 데이터베이스를 파싱 |
| Trae | `~/.config/tokscale/trae-cache/sessions/` | `%APPDATA%\tokscale\trae-cache\sessions\` | `tokscale trae sync`로 한 번 동기화; 설치된 Trae IDE 또는 Trae Solo 데스크톱 앱에서 자격 증명 자동 발견 |
| Warp/Oz | `~/.config/tokscale/warp-cache/usage.json` | `%APPDATA%\tokscale\warp-cache\usage.json` | `tokscale warp sync`로 동기화; 집계된 요청 수와 비용만, 토큰 트랜스크립트 없음 |
| Grok Build | `~/.grok/sessions/` | `%USERPROFILE%\.grok\sessions\` | `GROK_HOME` 환경변수로 설정 가능; `updates.jsonl` 세션 업데이트 파싱 |
| Jcode | `~/.jcode/sessions/` | `%USERPROFILE%\.jcode\sessions\` | `JCODE_HOME` 환경변수로 설정 가능; `session_*.json` 스냅샷과 `session_*.journal.jsonl` 사이드카 파싱 |
| MiMo Code | `~/.local/share/mimocode/` | `%USERPROFILE%\.local\share\mimocode\` | XDG 데이터 디렉토리 사용; SQLite 데이터베이스 `mimocode.db` |
| Gajae-Code | `~/.gjc/agent/sessions/` | `%USERPROFILE%\.gjc\agent\sessions\` | `GJC_CODING_AGENT_DIR`로 설정 가능 (`GJC_CONFIG_DIR`/`PI_CONFIG_DIR`도 지원; Linux/macOS에서는 `$XDG_DATA_HOME/gjc/sessions/`도 확인) |
| Junie | `~/.junie/sessions/` | `%USERPROFILE%\.junie\sessions\` | 모든 플랫폼에서 동일한 home 상대 경로 사용; `events.jsonl` 사용 이벤트 파싱 |
| ZCode | `~/.zcode/cli/db/db.sqlite` 및 `~/.zcode/projects/` | `%USERPROFILE%\.zcode\cli\db\db.sqlite` 및 `%USERPROFILE%\.zcode\projects\` | v2 SQLite 모델 사용량과 레거시 `*.jsonl` 세션 트랜스크립트 파싱; Z.ai의 GLM 모델용 ADE |
| OpenCodeReview | `~/.opencodereview/sessions/` | `%USERPROFILE%\.opencodereview\sessions\` | `*.jsonl` 세션 트랜스크립트 파싱; Alibaba의 AI 코드 리뷰 도구 |
| CodeBuddy | `~/.codebuddy/projects/` + 확장 프로그램 로그 | `%USERPROFILE%\.codebuddy\projects\` + CodeBuddy / VS Code 확장 프로그램 로그 | CodeBuddy CLI, IDE, VS Code 플러그인 토큰 사용량 파싱 |
| WorkBuddy | `~/.workbuddy/projects/` + `~/.workbuddy/workbuddy.db` | `%USERPROFILE%\.workbuddy\projects\` + `%USERPROFILE%\.workbuddy\workbuddy.db` | WorkBuddy 토큰 사용량 파싱, 집계 SQLite 데이터베이스를 폴백으로 사용 |
| Synthetic | 다른 소스에서 재귀속 | 다른 소스에서 재귀속 | `hf:` 모델 접두사 + `synthetic` provider 감지 |

> **참고**: Windows에서 `~`는 `%USERPROFILE%`로 확장됩니다 (예: `C:\Users\사용자이름`). 이러한 도구들은 `%APPDATA%`와 같은 Windows 기본 경로 대신 크로스 플랫폼 일관성을 위해 의도적으로 Unix 스타일 경로(`.local/share` 등)를 사용합니다.

#### Windows 전용 설정

Tokscale은 다음 위치에 설정을 저장합니다:
- **TUI 설정**: `%APPDATA%\tokscale\settings.json` (플랫폼 기본값. `TOKSCALE_CONFIG_DIR`로 오버라이드 가능)
- **캐시**: `%APPDATA%\tokscale\cache\` (통합 캐시 루트)
- **레거시 캐시 경로**: 이전 릴리스의 `%USERPROFILE%\.cache\tokscale\` 같은 분리된 경로가 새 경로로 다시 생성 가능한 데이터가 기록될 때까지 남아 있을 수 있습니다.
- **Cursor 자격 증명**: `%USERPROFILE%\.config\tokscale\cursor-credentials.json`
- **Trae 자격 증명 및 동기화된 사용량**: `%APPDATA%\tokscale\trae-cache\`
- **Tokscale 계정 자격 증명**: `%USERPROFILE%\.config\tokscale\credentials.json`

## 세션 데이터 보존

기본적으로 일부 AI 코딩 어시스턴트는 오래된 세션 파일을 자동으로 삭제합니다. 정확한 추적을 위해 사용 기록을 보존하려면 정리 기간을 비활성화하거나 연장하세요.

| 플랫폼 | 기본값 | 설정 파일 | 비활성화 설정 | 출처 |
|----------|---------|-------------|-------------------|--------|
| Claude Code | **⚠️ 30일** | `~/.claude/settings.json` | `"cleanupPeriodDays": 9999999999` | [문서](https://docs.anthropic.com/en/docs/claude-code/settings) |
| Gemini CLI | 비활성화됨 | `$GEMINI_CLI_HOME/settings.json` (폴백: `~/.gemini/settings.json`) | `"general.sessionRetention.enabled": false` | [문서](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/session-management.md) |
| Codex CLI | 비활성화됨 | N/A | 정리 기능 없음 | [#6015](https://github.com/openai/codex/issues/6015) |
| OpenCode | 비활성화됨 | N/A | 정리 기능 없음 | [#4980](https://github.com/sst/opencode/issues/4980) |

### Claude Code

**기본값**: 30일 정리 기간

`~/.claude/settings.json`에 추가:
```json
{
  "cleanupPeriodDays": 9999999999
}
```

> 매우 큰 값 (예: `9999999999`일 ≈ 2700만 년)을 설정하면 사실상 정리가 비활성화됩니다.

### Gemini CLI

**기본값**: 정리 비활성화됨 (세션이 영구 보존)

정리를 활성화했다가 비활성화하려면 `$GEMINI_CLI_HOME/settings.json` (폴백: `~/.gemini/settings.json`)에서 제거하거나 `enabled: false`로 설정:
```json
{
  "general": {
    "sessionRetention": {
      "enabled": false
    }
  }
}
```

또는 매우 긴 보존 기간 설정:
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

**기본값**: 자동 정리 없음 (세션이 영구 보존)

Codex CLI는 내장 세션 정리가 없습니다. `~/.codex/sessions/`의 세션은 무기한 유지됩니다.

> **참고**: 이에 대한 기능 요청이 있습니다: [#6015](https://github.com/openai/codex/issues/6015)

### OpenCode

**기본값**: 자동 정리 없음 (세션이 영구 보존)

OpenCode는 내장 세션 정리가 없습니다. `~/.local/share/opencode/storage/`의 세션은 무기한 유지됩니다.

> **참고**: [#4980](https://github.com/sst/opencode/issues/4980) 참조

---

## 데이터 소스

### OpenCode

위치: `~/.local/share/opencode/opencode.db` (v1.2+) 또는 `storage/message/{sessionId}/*.json` (레거시)

OpenCode 1.2+는 세션을 SQLite에 저장합니다. Tokscale은 SQLite를 먼저 읽고, 이전 버전의 경우 레거시 JSON 파일로 폴백합니다.

OpenCode는 빌드된 릴리스 채널에 따라 DB 파일명을 결정합니다: `latest`, `beta` 채널은 `opencode.db`를 사용하고, 나머지 채널은 `opencode-<channel>.db` (예: `opencode-stable.db`, `opencode-nightly.db`)를 사용합니다. Tokscale은 모든 변형을 스캔하므로 여러 채널을 함께 사용하는 경우에도 통합된 뷰를 제공합니다.

`OPENCODE_DB`를 `~/.local/share/opencode` 밖의 파일로 지정해 opencode를 실행한 경우, tokscale이 매 실행마다 찾을 수 있도록 `~/.config/tokscale/settings.json`에 절대 경로를 추가하세요:

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

경로는 자동 발견과 병합되고, 정규 경로 기준으로 중복 제거되며, 존재하지 않는 항목은 조용히 건너뜁니다 (오래된 설정이 스캔을 깨뜨리지 않도록). `opencode.db-wal`, `opencode.db-shm` 및 기타 SQLite 사이드카는 거부됩니다.

Tokscale의 기본 home-root 위치 밖에 세션을 보관하는 경우, 클라이언트별 추가 스캔 루트를 영구적으로 지정할 수도 있습니다:

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

이는 프로젝트 단위 `.codex` 디렉터리, 가져온 히스토리, 그리고 기본 `$HERMES_HOME/state.db`나 `~/.hermes/state.db` 위치 밖의 Hermes 프로필 데이터베이스에 유용합니다. Tokscale은 여전히 기본 루트를 스캔한 다음, 그 위에 `scanner.extraScanPaths`와 `TOKSCALE_EXTRA_DIRS`를 정규 경로 중복 제거와 함께 병합합니다. 워크스페이스 전체를 자동 발견하지는 않습니다.

각 메시지 포함 내용:
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

위치: `~/.claude/projects/{projectPath}/*.jsonl` 및 `~/.claude/transcripts/*.jsonl`

어시스턴트 메시지의 사용량 데이터를 포함하는 JSONL 형식:
```json
{"type": "assistant", "message": {"model": "claude-sonnet-4-20250514", "usage": {"input_tokens": 1234, "output_tokens": 567, "cache_read_input_tokens": 890}}, "timestamp": "2024-01-01T00:00:00Z"}
```

`~/.claude/transcripts/` 아래의 래퍼 트랜스크립트 파일은 실제 Claude 사용량 메타데이터를 포함할 때만 계산됩니다. 사용자/도구 이벤트는 있지만 `usage` 블록이 없는 파일은 추정하지 않고 건너뜁니다.

Tokscale의 `claude` 클라이언트는 Claude Code 토큰 회계이며, Claude Desktop 채팅 회계가 아닙니다. Claude Desktop은 `~/Library/Application Support/Claude` 같은 위치에 앱 데이터를 저장하지만, Anthropic은 소비자용 데스크톱 채팅이나 채팅 기록 내보내기에 대한 안정적인 로컬 메시지별 토큰 원장을 문서화하지 않습니다. Claude Desktop 데이터는 존재하지만 Claude Code JSONL 루트만 스캔 가능한 경우 `tokscale clients`를 실행하면 진단 정보를 볼 수 있습니다. `tokscale usage`는 Claude Code 자격 증명으로부터 best-effort Claude 구독 할당량 막대를 표시할 수 있는 반면, 조직/API 사용량은 Anthropic의 Admin Usage and Cost API에 속하며 로컬 트랜스크립트 스캔과는 의도적으로 분리되어 있습니다.

### Codex CLI

위치: `~/.codex/sessions/*.jsonl`

`token_count` 이벤트가 있는 이벤트 기반 형식:
```json
{"type": "event_msg", "payload": {"type": "token_count", "info": {"last_token_usage": {"input_tokens": 1234, "output_tokens": 567}}}}
```

### Copilot CLI

위치: `~/.copilot/otel/*.jsonl` 또는 `COPILOT_OTEL_FILE_EXPORTER_PATH`에 명시된 경로

Copilot 지원은 파일로 내보낸 OpenTelemetry JSONL을 읽습니다. Copilot을 실행하기 전에 활성화하세요:

```bash
export COPILOT_OTEL_ENABLED=true
export COPILOT_OTEL_EXPORTER_TYPE=file
mkdir -p "$HOME/.copilot/otel"
export COPILOT_OTEL_FILE_EXPORTER_PATH="$HOME/.copilot/otel/copilot-otel-$(date +%Y%m%d-%H%M%S).jsonl"
```

PowerShell:

```powershell
$otelDir = "$HOME/.copilot/otel"
New-Item -ItemType Directory -Force -Path $otelDir | Out-Null
$env:COPILOT_OTEL_ENABLED = "true"
$env:COPILOT_OTEL_EXPORTER_TYPE = "file"
$env:COPILOT_OTEL_FILE_EXPORTER_PATH = Join-Path $otelDir ("copilot-otel-{0}.jsonl" -f (Get-Date -Format "yyyyMMdd-HHmmss"))
```

타임스탬프가 포함된 파일명을 사용하면 각 Copilot 세션이 하나의 거대한 OTEL 로그에 누적되지 않고 새 파일에 기록됩니다.

Tokscale은 `chat` span을 토큰 집계의 출처로 취급하고, 도구 span과 누적 메트릭은 1단계에서 무시합니다:

```json
{"type":"span","name":"chat gpt-5.4-mini","attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"session-id","gen_ai.usage.input_tokens":1234,"gen_ai.usage.output_tokens":567,"gen_ai.usage.cache_read.input_tokens":890,"gen_ai.usage.reasoning.output_tokens":123}}
```

> Copilot의 OTEL 페이로드는 현재 안정적인 워크스페이스 메타데이터를 노출하지 않으므로, Copilot 행은 워크스페이스 속성 없이 표시될 수 있습니다. Tokscale은 가능한 경우 보고된 모델로부터 해당 행의 가격을 책정하며, `github.copilot.cost`를 직접 신뢰하지 않습니다.

### Gemini CLI

위치: `$GEMINI_CLI_HOME/tmp/{projectHash}/chats/*.json` (폴백: `~/.gemini/tmp/{projectHash}/chats/*.json`)

메시지 배열을 포함한 세션 파일:
```json
{
  "sessionId": "xxx",
  "messages": [
    {"type": "gemini", "model": "gemini-2.5-pro", "tokens": {"input": 1234, "output": 567, "cached": 890, "thoughts": 123}}
  ]
}
```

### Cursor IDE

위치: `~/.config/tokscale/cursor-cache/usage*.csv` (Cursor API를 통해 동기화)

Cursor 데이터는 세션 토큰을 사용하여 Cursor API에서 가져와 로컬에 캐시됩니다. Tokscale은 리포트를 위해 해당 캐시 파일을 읽으며, 로컬 `~/.cursor` 세션 데이터는 파싱하지 않습니다. 설정 안내는 [Cursor IDE 명령어](#cursor-ide-명령어)를 참조하세요.

### Antigravity

위치: `~/.config/tokscale/antigravity-cache/sessions/*.jsonl` (로컬 Antigravity 언어 서버 RPC를 통해 동기화)

Antigravity 데이터는 루트 명령에서 자동으로 가져오지 않습니다. Antigravity가 활성화된 에디터가 열려 있는 동안 `tokscale antigravity sync`를 실행해 로컬 캐시를 새로 고친 다음, 캐시된 JSONL 아티팩트에 대해 일반적인 tokscale 리포트와 필터를 사용하세요.

### Trae

위치: `~/.config/tokscale/trae-cache/sessions/*.json` (공식 사용량 API를 통해 동기화)

Trae 데이터는 루트 명령에서 자동으로 가져오지 않습니다. 먼저 `tokscale trae login`을 실행한 뒤, 리포트 전에 `tokscale trae sync` 또는 `tokscale trae sync --since 30`을 실행하세요. Tokscale은 동기화된 API dump를 세션 수준 레코드로 파싱하고 Trae가 반환한 비용 합계를 보존합니다.

### Warp/Oz

위치: `~/.config/tokscale/warp-cache/usage.json` (인증된 GraphQL API를 통해 동기화)

Warp/Oz 데이터는 루트 명령으로 자동으로 가져오지 않습니다. 리포트 전에 `tokscale warp login`을 실행한 다음 `tokscale warp sync`를 실행하세요. Warp는 토큰에 귀속된 로컬 트랜스크립트를 노출하지 않으므로, Tokscale은 집계된 요청 수와 지출만 기록합니다.

### Grok Build

위치: `$GROK_HOME/sessions/*/*/updates.jsonl` (폴백: `~/.grok/sessions/*/*/updates.jsonl`)

Grok Build 데이터는 로컬 세션 업데이트에서 직접 파싱됩니다. 현재 로그는 안정적인 input/output 분리 없이 누적 `totalTokens` 카운터를 노출하므로, Tokscale은 턴별 양수 증가분을 input 토큰으로 기록합니다. `grok-composer-2.5-fast`는 전용 공개 가격이 생길 때까지 Composer 2.5 Fast 가격 override에 임시 매핑됩니다.

### Jcode

위치: `$JCODE_HOME/sessions/session_*.json` (폴백: `~/.jcode/sessions/session_*.json`) 및 매칭되는 `session_*.journal.jsonl` 사이드카.

Jcode 데이터는 로컬 세션 스냅샷에서 직접 파싱됩니다. Tokscale은 다른 클라이언트 신원을 위장하지 않고 어시스턴트의 `messages[].token_usage` 필드(`input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`, `reasoning_output_tokens`)를 읽습니다. 매칭되는 저널 사이드카는 중복 제거 전에 동일한 세션 스트림으로 병합되므로, Jcode가 스냅샷에 체크포인트로 반영하기 전까지 최근에 추가된 메시지도 포함됩니다. 재생(replay) 중복 제거에는 안정적인 메시지 ID를 사용하며, ID가 없는 잘못된/커스텀 레코드는 범위가 한정된 폴백 키를 사용합니다.

### OpenClaw

위치: `~/.openclaw/agents/*/sessions/sessions.json` (레거시 경로도 스캔: `~/.clawdbot/`, `~/.moltbot/`, `~/.moldbot/`)

JSONL 세션 파일을 가리키는 인덱스 파일:
```json
{
  "agent:main:main": {
    "sessionId": "uuid",
    "sessionFile": "/path/to/session.jsonl"
  }
}
```

model_change 이벤트와 어시스턴트 메시지가 포함된 세션 JSONL 형식:
```json
{"type":"model_change","provider":"openai-codex","modelId":"gpt-5.2"}
{"type":"message","message":{"role":"assistant","usage":{"input":1660,"output":55,"cacheRead":108928,"cost":{"total":0.02}},"timestamp":1769753935279}}
```

### Hermes Agent

위치: `$HERMES_HOME/state.db` (폴백: `~/.hermes/state.db`) 및 표준 프로필 데이터베이스 `$HERMES_HOME/profiles/*/state.db` (`HERMES_HOME`이 활성 프로필을 가리키는 경우 형제 `~/.hermes/profiles/*/state.db`)

Hermes는 세션 수준 사용량을 SQLite `sessions` 테이블에 저장합니다. Tokscale은 `model`이 존재하고 토큰 또는 비용 합계가 0이 아닌 행을 가져오며, `started_at`을 타임스탬프로 사용하고, `message_count`를 보존하며, `actual_cost_usd`를 `estimated_cost_usd`보다 우선합니다.

### Pi

위치: `~/.pi/agent/sessions/<encoded-cwd>/*.jsonl` 및 `~/.omp/agent/sessions/<encoded-cwd>/*.jsonl` ([Oh My Pi](https://github.com/can1357/oh-my-pi))

세션 헤더와 메시지 항목을 포함하는 JSONL 형식:
```json
{"type":"session","id":"pi_ses_001","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"claude-3-5-sonnet","provider":"anthropic","usage":{"input":100,"output":50,"cacheRead":10,"cacheWrite":5,"totalTokens":165}}}
```

### Kimi CLI

위치: `~/.kimi/sessions/{GROUP_ID}/{SESSION_UUID}/wire.jsonl`

StatusUpdate 메시지를 포함하는 wire.jsonl 형식:
```json
{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983426.420942, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 1562, "output": 2463, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "chatcmpl-xxx"}}}
```

### Kimi Code

위치: `~/.kimi-code/sessions/{WORKDIR}/{SESSION_UUID}/agents/{AGENT}/wire.jsonl`
```json
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":1163,"output":352,"inputCacheRead":22272,"inputCacheCreation":0},"usageScope":"turn","time":1780410897480}
```

### Qwen CLI

위치: `~/.qwen/projects/{PROJECT_PATH}/chats/{CHAT_ID}.jsonl`

형식: JSONL — 줄당 하나의 JSON 객체, 각각 `type`, `model`, `timestamp`, `sessionId`, `usageMetadata` 필드 포함.

토큰 필드 (`usageMetadata`에서):
- `promptTokenCount` → 입력 토큰
- `candidatesTokenCount` → 출력 토큰
- `thoughtsTokenCount` → 추론/사고 토큰
- `cachedContentTokenCount` → 캐시된 입력 토큰

### Roo Code

위치:
- 로컬: `~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks/{TASK_ID}/ui_messages.json`
- 서버 (최선 노력): `~/.vscode-server/data/User/globalStorage/rooveterinaryinc.roo-cline/tasks/{TASK_ID}/ui_messages.json`

각 작업 디렉토리에는 모델/에이전트 메타데이터에 사용되는 `<environment_details>` 블록이 포함된 `api_conversation_history.json`이 포함될 수 있습니다.

`ui_messages.json`은 UI 이벤트 배열입니다. Tokscale은 다음만 계산합니다:
- `type == "say"`
- `say == "api_req_started"`

`text` 필드는 토큰/비용 메타데이터를 포함하는 JSON입니다:
```json
{
  "type": "say",
  "say": "api_req_started",
  "ts": "2026-02-18T12:00:00Z",
  "text": "{\"cost\":0.12,\"tokensIn\":100,\"tokensOut\":50,\"cacheReads\":20,\"cacheWrites\":5,\"apiProtocol\":\"anthropic\"}"
}
```

### Kilo

위치:
- 로컬: `~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/{TASK_ID}/ui_messages.json`
- 서버 (최선 노력): `~/.vscode-server/data/User/globalStorage/kilocode.kilo-code/tasks/{TASK_ID}/ui_messages.json`

Kilo는 Roo Code와 동일한 작업 로그 형식을 사용합니다. Tokscale은 동일한 규칙을 적용합니다:
- `ui_messages.json`에서 `say/api_req_started` 이벤트만 계산
- `text` JSON에서 `tokensIn`, `tokensOut`, `cacheReads`, `cacheWrites`, `cost`, `apiProtocol` 파싱
- 사용 가능한 경우 인접한 `api_conversation_history.json`에서 모델/에이전트 메타데이터 보강

### Cline

위치:
- Linux 데스크톱 VS Code: `~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/{TASK_ID}/ui_messages.json`
- macOS 데스크톱 VS Code: `~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/{TASK_ID}/ui_messages.json`
- Windows 데스크톱 VS Code: `%APPDATA%\Code\User\globalStorage\saoudrizwan.claude-dev\tasks\{TASK_ID}\ui_messages.json`
- 서버 (최선 노력): `~/.vscode-server/data/User/globalStorage/saoudrizwan.claude-dev/tasks/{TASK_ID}/ui_messages.json`

Cline은 Roo Code와 Kilo가 포크한 원본(upstream) 프로젝트로, 동일한 VS Code globalStorage 작업 로그 형식을 사용합니다. Tokscale은 동일한 규칙을 적용합니다:
- `ui_messages.json`에서 `say/api_req_started` 이벤트만 계산
- `text` JSON에서 `tokensIn`, `tokensOut`, `cacheReads`, `cacheWrites`, `cost`, `apiProtocol` 파싱
- 사용 가능한 경우 인접한 `api_conversation_history.json`에서 모델/에이전트 메타데이터 보강

### Mux

위치:
 `~/.mux/sessions/{WORKSPACE_ID}/session-usage.json`

Mux는 세션별 누적 토큰 사용량을 `session-usage.json` 파일에 저장합니다. 각 파일에는 모델별 토큰 분류가 포함된 `byModel` 맵이 있습니다:
 `input`, `cached` (캐시 읽기), `cacheCreate` (캐시 쓰기), `output`, `reasoning`
 모델명은 `provider:model` 형식을 사용합니다 (예: `anthropic:claude-opus-4-6`) — tokscale은 모델 식별을 위해 provider 접두사를 제거합니다
 하위 에이전트 사용량은 Mux에 의해 자동으로 상위 세션에 합산되므로 중복 집계가 없습니다

### Kilo CLI

위치: `~/.local/share/kilo/kilo.db`

Kilo CLI는 OpenCode와 유사한 SQLite 데이터베이스에 세션 데이터를 저장합니다. 각 메시지 행에는 모델 및 공급자 속성과 함께 메시지별 토큰 분류(입력, 출력, 캐시 읽기/쓰기, 추론)가 포함됩니다.

### Crush

위치: `$XDG_DATA_HOME/crush/projects.json`를 통해 발견되는 프로젝트별 SQLite 데이터베이스 (기본값: `~/.local/share/crush/projects.json`)

Crush는 프로젝트별 SQLite 데이터베이스(`crush.db`)에 사용량을 저장합니다. Crush는 신뢰할 수 있는 메시지별 또는 모델별 토큰 집계를 제공하지 않으므로, Tokscale은 루트 세션의 세션 수준 비용 합계만 가져옵니다. 레코드는 `model=session-total`로 표시되며 토큰 분류는 0입니다.

### Goose

위치: `~/.local/share/goose/sessions/sessions.db` (`~/Library/Application Support/goose/`, `~/Library/Application Support/Block/goose/`, `~/.local/share/Block/goose/`도 스캔; `GOOSE_PATH_ROOT`로 오버라이드 가능)

Goose는 세션별 사용량을 SQLite `sessions.db`에 저장합니다. Tokscale은 `model_config_json`에서 모델, `provider_name`에서 공급자, 그리고 세션별로 누적된 입력/출력 토큰 합계를 추출합니다. 추론 토큰은 해당 컬럼이 채워져 있을 때 추정됩니다.

### Codebuff

위치: `~/.config/manicode/projects/<project>/chats/<chatId>/chat-messages.json` (`manicode-dev` 및 `manicode-staging` 채널도 스캔; `CODEBUFF_DATA_DIR`로 오버라이드 가능)

Codebuff(이전 Manicode)는 채팅별 JSON 파일을 저장합니다. Tokscale은 `metadata.usage`, `metadata.codebuff.usage` 및 run-state의 `messageHistory[*].providerOptions` 폴백에서 토큰 사용량을 파싱하며, 부분적으로 갱신된 최신 항목이 실제 토큰 카운트를 가진 이전 항목을 가리지 않도록 히스토리를 역순으로 순회합니다. 메시지별 타임스탬프가 없을 때는 chat-id 디렉토리 이름, 마지막으로 파일 mtime으로 폴백합니다.

### Gajae-Code (gjc)

위치: `~/.gjc/agent/sessions/<project-slug>/*.jsonl` (`GJC_CODING_AGENT_DIR`로 에이전트 디렉토리 오버라이드 가능; `GJC_CONFIG_DIR`/`PI_CONFIG_DIR`에 `agent/sessions`를 붙인 경로도 확인하며, Linux/macOS에서는 `$XDG_DATA_HOME/gjc/sessions/`로 단축된 경로도 지원). 뎁스-2 패스별 서브에이전트 전사본(`<slug>/<session>/N-*.jsonl`)도 탐색합니다.

세션 헤더와 메시지 항목으로 구성된 JSONL 형식입니다. Tokscale은 어시스턴트 메시지만 처리하며, 메시지당 `usage.cost.total`(USD)이 있으면 그 값을 우선 사용하고, 없을 때만 토큰에서 비용을 재계산합니다:
```json
{"type":"session","id":"S1","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/work/proj"}
{"type":"message","id":"M1","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"claude-sonnet-4","provider":"anthropic","usage":{"input":1000,"output":500,"cacheRead":0,"cacheWrite":0,"totalTokens":1500,"cost":{"input":0.1,"output":0.2,"total":0.3}}}}
```
메시지는 `<세션 id>:<메시지 id>`로 중복 제거됩니다(결정론적 폴백 포함). 재실행된 뎁스-1/뎁스-2 전사본은 한 번만 집계됩니다. `service_tier_change` 항목과 잘못된 형식의 줄은 줄 단위로 건너뜁니다.

### Synthetic (synthetic.new)

Synthetic은 기존 에이전트 세션을 후처리하여 재귀속합니다. `hf:` 접두사 모델 ID 또는 `synthetic` / `glhf` / `octofriend` provider를 감지하면 해당 메시지를 `synthetic` 소스로 처리합니다.

또한 `~/.local/share/octofriend/sqlite.db`를 감지해 토큰 정보가 있는 레코드를 파싱합니다.

### MiMo Code

위치: `~/.local/share/mimocode/mimocode.db` (XDG 데이터 디렉토리)

MiMo Code는 SQLite 데이터베이스에 세션 데이터를 저장합니다. Tokscale은 워크스페이스 컨텍스트를 위해 `session` 테이블과 조인된 `message` 테이블을 쿼리합니다:

```sql
SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root
FROM message m
LEFT JOIN session s ON s.id = m.session_id
WHERE json_extract(m.data, '$.role') = 'assistant'
  AND json_extract(m.data, '$.tokens') IS NOT NULL
```

`data` 컬럼은 JSON 형식이며 다음 토큰 관련 필드를 포함합니다:
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

Tokscale은 타임스탬프, 모델, 프로바이더, 토큰 수, 비용, 에이전트 이름의 지문을 사용하여 포크된 세션 간 메시지를 중복 제거합니다.

## 가격

Tokscale은 [LiteLLM의 가격 데이터베이스](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json)에서 실시간 가격을 가져옵니다.

**동적 폴백**: LiteLLM에 아직 없는 모델(예: 최근 출시된 모델)은 [OpenRouter의 엔드포인트 API](https://openrouter.ai/docs/api/api-reference/endpoints/list-endpoints)에서 자동으로 가격을 가져옵니다.

**Cursor 모델 가격**: LiteLLM과 OpenRouter 모두에 없는 최신 모델(예: `gpt-5.3-codex`)은 [Cursor 모델 문서](https://cursor.com/en-US/docs/models)에서 가져온 하드코딩 가격을 사용합니다. 이 오버라이드는 모든 업스트림 소스 다음에, 퍼지 매칭 이전에 확인되므로 실제 업스트림 가격이 사용 가능해지면 자동으로 양보합니다.

**Sakana Fugu 가격**: Fugu Ultra 비용은 Sakana가 공개한 종량제(pay-as-you-go) 요율로 추정하며, `fugu` 라우터 모델은 실제로 오케스트레이션한 기반 모델의 가변 요율이 곧 그 비용이므로 의도적으로 가격을 책정하지 않습니다.

**캐싱**: 가격 데이터는 1시간 TTL로 디스크에 캐시되어 빠른 시작을 보장합니다:
- LiteLLM 캐시: `~/.config/tokscale/cache/pricing-litellm.json`
- OpenRouter 캐시: `~/.config/tokscale/cache/pricing-openrouter.json` (지원 제공자의 모델에 대한 작성자 가격 정보를 캐시)

가격 포함 항목:
- 입력 토큰
- 출력 토큰
- 캐시 읽기 토큰 (할인)
- 캐시 쓰기 토큰
- 추론 토큰 (o1과 같은 모델용)
- 모델별 구간 가격 (예: 200k 또는 272k 토큰 이상)

## 기여

기여를 환영합니다! 다음 단계를 따르세요:

1. 저장소 포크
2. 기능 브랜치 생성 (`git checkout -b feature/amazing-feature`)
3. 변경 사항 작성
4. 테스트 실행 (`cd packages/core && bun run test:all`)
5. 변경 사항 커밋 (`git commit -m 'Add amazing feature'`)
6. 브랜치에 푸시 (`git push origin feature/amazing-feature`)
7. Pull Request 열기

### 개발 가이드라인

- 기존 코드 스타일 따르기
- 새로운 기능에 테스트 추가
- 필요에 따라 문서 업데이트
- 커밋은 집중적이고 원자적으로 유지

## 감사의 글

- 영감을 준 [ccusage](https://github.com/ryoppippi/ccusage), [viberank](https://github.com/sculptdotfun/viberank), [Isometric Contributions](https://github.com/jasonlong/isometric-contributions)
- 터미널 UI 프레임워크 [Ratatui](https://github.com/ratatui/ratatui)
- 반응형 렌더링을 위한 [Solid.js](https://www.solidjs.com/)
- 가격 데이터를 위한 [LiteLLM](https://github.com/BerriAI/litellm)
- Rust/Node.js 바인딩을 위한 [napi-rs](https://napi.rs/)
- 2D 그래프 참조를 위한 [github-contributions-canvas](https://github.com/sallar/github-contributions-canvas)

## 라이선스

<p align="center">
  <a href="https://github.com/junhoyeo">
    <img src=".github/assets/labtocat-on-spaceship.png" width="540">
  </a>
</p>

<p align="center">
  <strong>MIT © <a href="https://github.com/junhoyeo">Junho Yeo</a></strong>
</p>

이 프로젝트가 흥미롭다면 **스타(⭐)**를 눌러주세요.  
[GitHub에서 저를 팔로우](https://github.com/junhoyeo)하고 함께 빌드해도 좋아요. (이미 1.1k+명이 탑승해 있어요!)
