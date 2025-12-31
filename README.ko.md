<!-- <CENTERED SECTION FOR GITHUB DISPLAY> -->

<div align="center">

[![Tokscale](./.github/assets/hero.png)](https://tokscale.ai)

</div>

> 여러 플랫폼에서 AI 코딩 어시스턴트의 **토큰 사용량과 비용**을 추적하는 고성능 CLI 도구 및 시각화 대시보드

<div align="center">

[![GitHub Release](https://img.shields.io/github/v/release/junhoyeo/tokscale?color=0073FF&labelColor=black&logo=github&style=flat-square)](https://github.com/junhoyeo/tokscale/releases)
[![npm Downloads](https://img.shields.io/npm/dt/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://www.npmjs.com/package/tokscale)
[![GitHub Contributors](https://img.shields.io/github/contributors/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/graphs/contributors)
[![GitHub Forks](https://img.shields.io/github/forks/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/network/members)
[![GitHub Stars](https://img.shields.io/github/stars/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/stargazers)
[![GitHub Issues](https://img.shields.io/github/issues/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/issues)
[![License](https://img.shields.io/badge/license-MIT-white?labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/blob/master/LICENSE)

[English](README.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | [简体中文](README.zh-cn.md)

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

| 로고 | 클라이언트 | 데이터 위치 | 지원 여부 |
|------|----------|---------------|-----------|
| <img width="48px" src=".github/assets/client-opencode.png" alt="OpenCode" /> | [OpenCode](https://github.com/sst/opencode) | `~/.local/share/opencode/storage/message/` | ✅ 지원 |
| <img width="48px" src=".github/assets/client-claude.jpg" alt="Claude" /> | [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `~/.claude/projects/` | ✅ 지원 |
| <img width="48px" src=".github/assets/client-openai.jpg" alt="Codex" /> | [Codex CLI](https://github.com/openai/codex) | `~/.codex/sessions/` | ✅ 지원 |
| <img width="48px" src=".github/assets/client-gemini.png" alt="Gemini" /> | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `~/.gemini/tmp/*/chats/` | ✅ 지원 |
| <img width="48px" src=".github/assets/client-cursor.jpg" alt="Cursor" /> | [Cursor IDE](https://cursor.com/) | `~/.config/tokscale/cursor-cache/`를 통한 API 동기화 | ✅ 지원 |
| <img width="48px" src=".github/assets/client-amp.png" alt="Amp" /> | [Amp (AmpCode)](https://ampcode.com/) | `~/.local/share/amp/threads/` | ✅ 지원 |
| <img width="48px" src=".github/assets/client-droid.png" alt="Droid" /> | [Droid (Factory Droid)](https://factory.ai/) | `~/.factory/sessions/` | ✅ 지원 |

[🚅 LiteLLM의 가격 데이터](https://github.com/BerriAI/litellm)를 사용해 **실시간 비용 계산**을 제공합니다. 구간별 가격 모델(대용량 컨텍스트 등)과 **캐시 토큰 할인**도 지원합니다.

### 왜 "Tokscale"인가요?

이 프로젝트는 **[카르다쇼프 척도(Kardashev Scale)](https://ko.wikipedia.org/wiki/%EC%B9%B4%EB%A5%B4%EB%8B%A4%EC%87%BC%ED%94%84_%EC%B2%99%EB%8F%84)**에서 영감을 받았습니다. 카르다쇼프 척도는 문명의 기술 수준을 **에너지 소비량**으로 분류합니다. 유형 I 문명은 행성에서 사용 가능한 모든 에너지를 활용하고, 유형 II는 항성의 전체 출력을 포착하며, 유형 III는 은하 전체의 에너지를 통제합니다.

AI 지원 개발 시대에 **토큰은 새로운 에너지**입니다. 토큰은 우리의 사고력을 구동하고, 생산성을 높이며, 창의적 결과물을 이끌어냅니다. 카르다쇼프 척도가 우주적 규모에서 에너지 소비를 추적하듯, Tokscale은 AI 증강 개발의 단계를 올라가며 **토큰 소비를 측정하고 시각화**합니다. 가볍게 쓰는 사용자든 매일 수백만 개의 토큰을 소비하는 파워 유저든, Tokscale은 “내가 어디에서 무엇을 얼마나 쓰고 있는지”를 분명하게 보여줍니다.

## 기능

- **인터랙티브 TUI 모드** - OpenTUI 기반의 터미널 UI (기본 모드)
  - 4개 뷰: 개요, 모델, 일별, 통계
  - 키보드 및 마우스 지원
  - 9가지 테마의 GitHub 스타일 기여 그래프
  - 실시간 필터링 및 정렬
  - 깜빡임 없는 렌더링 (네이티브 Zig 엔진)
- **멀티 플랫폼 지원** - OpenCode, Claude Code, Codex CLI, Cursor IDE, Gemini CLI 사용량 통합 추적
- **실시간 가격 반영** - LiteLLM에서 최신 가격을 가져와(디스크 캐시 1시간) 비용 계산
- **상세 분석** - 입력, 출력, 캐시 읽기/쓰기, 추론 토큰까지 추적
- **네이티브 Rust 코어** - 모든 파싱과 집계를 Rust로 처리해 최대 10배 빠른 성능
- **웹 시각화** - 2D 및 3D 뷰의 인터랙티브 기여 그래프
- **유연한 필터링** - 플랫폼, 날짜 범위 또는 연도별 필터링
- **JSON 내보내기** - 외부 시각화 도구/자동화용 데이터 생성
- **소셜 플랫폼** - 사용량 공유, 리더보드 경쟁, 공개 프로필 조회

## 설치

### 빠른 시작

```bash
# Bun 설치 (아직 설치하지 않은 경우)
curl -fsSL https://bun.sh/install | bash

# bunx로 바로 실행
bunx tokscale@latest
```

이게 전부입니다! 별도 설정 없이 바로 완전한 인터랙티브 TUI 경험을 제공합니다.

> **[Bun](https://bun.sh/) 필요**: 인터랙티브 TUI는 깜빡임 없는 렌더링을 위해 OpenTUI의 네이티브 Zig 모듈을 사용하며, 이는 Bun 런타임이 필요합니다.

> **패키지 구조**: `tokscale`은 `@tokscale/cli`를 설치하는 별칭 패키지입니다 ([`swc`](https://www.npmjs.com/package/swc)처럼). 둘 다 네이티브 Rust 코어 (`@tokscale/core`)가 포함된 동일한 CLI를 설치합니다.

### 사전 요구사항

- [Bun](https://bun.sh/) (필수)
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

- **4개 뷰**: 개요 (차트 + 상위 모델), 모델, 일별, 통계 (기여 그래프)
- **키보드 내비게이션**:
  - `1-4` 또는 `←/→/Tab`: 뷰 전환
  - `↑/↓`: 목록 탐색
  - `c/n/t`: 비용/이름/토큰별 정렬
  - `1-5`: 소스 토글 (OpenCode/Claude/Codex/Cursor/Gemini)
  - `p`: 9가지 색상 테마 순환
  - `r`: 데이터 새로고침
  - `e`: JSON으로 내보내기
  - `q`: 종료
- **마우스 지원**: 탭, 버튼, 필터 클릭
- **테마**: Green, Halloween, Teal, Blue, Pink, Purple, Orange, Monochrome, YlGnBu
- **설정 저장**: 테마 설정이 `~/.config/tokscale/tui-settings.json`에 저장됨

### 플랫폼별 필터링

```bash
# OpenCode 사용량만 표시
tokscale --opencode

# Claude Code 사용량만 표시
tokscale --claude

# Codex CLI 사용량만 표시
tokscale --codex

# Gemini CLI 사용량만 표시
tokscale --gemini

# Cursor IDE 사용량만 표시 (먼저 `tokscale cursor login` 필요)
tokscale --cursor

# 필터 조합
tokscale --opencode --claude
```

### 날짜 필터링

날짜 필터는 리포트를 생성하는 모든 명령어에서 작동합니다 (`tokscale`, `tokscale models`, `tokscale monthly`, `tokscale graph`):

```bash
# 빠른 날짜 단축키
tokscale --today              # 오늘만
tokscale --week               # 최근 7일
tokscale --month              # 이번 달

# 사용자 정의 날짜 범위 (포함, 로컬 타임존)
tokscale --since 2024-01-01 --until 2024-12-31

# 연도별 필터
tokscale --year 2024

# 다른 옵션과 조합
tokscale models --week --claude --json
tokscale monthly --month --benchmark
```

> **참고**: 날짜 필터는 로컬 타임존을 사용합니다. `--since`와 `--until` 모두 해당 날짜를 포함합니다.

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
```

**조회 전략:**

가격 조회는 다단계 해석 전략을 사용합니다:

1. **정확한 일치** - LiteLLM/OpenRouter 데이터베이스에서 직접 조회
2. **별칭 해석** - 친숙한 이름 해석 (예: `big-pickle` → `glm-4.7`)
3. **티어 접미사 제거** - 품질 티어 제거 (`gpt-5.2-xhigh` → `gpt-5.2`)
4. **버전 정규화** - 버전 형식 처리 (`claude-3-5-sonnet` ↔ `claude-3.5-sonnet`)
5. **프로바이더 접두사 매칭** - 일반 접두사 시도 (`anthropic/`, `openai/` 등)
6. **퍼지 매칭** - 부분 모델 이름에 대한 단어 경계 매칭

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

# 로그인한 사용자 확인
tokscale whoami

# 사용량 데이터를 리더보드에 제출
tokscale submit

# 필터와 함께 제출
tokscale submit --opencode --claude --since 2024-01-01

# 제출될 내용 미리보기 (드라이 런)
tokscale submit --dry-run

# 로그아웃
tokscale logout
```

<img alt="CLI Submit" src="./.github/assets/cli-submit.png" />

### Cursor IDE 명령어

Cursor IDE는 세션 토큰을 통한 별도의 인증이 필요합니다 (소셜 플랫폼 로그인과 다름):

```bash
# Cursor 로그인 (브라우저에서 세션 토큰 필요)
tokscale cursor login

# Cursor 인증 상태 및 세션 유효성 확인
tokscale cursor status

# Cursor 로그아웃 (저장된 자격 증명 제거)
tokscale cursor logout
```

**자격 증명 저장**: Cursor 세션 토큰은 `~/.config/tokscale/cursor-credentials.json`에 저장됩니다. 사용량 데이터는 `~/.config/tokscale/cursor-cache/`에 캐시됩니다.

**Cursor 세션 토큰 얻는 방법:**
1. 브라우저에서 https://www.cursor.com/settings 열기
2. 개발자 도구 열기 (F12)
3. **옵션 A - Network 탭**: 페이지에서 아무 동작을 하고, `cursor.com/api/*`에 대한 요청을 찾아, Request Headers에서 `Cookie` 헤더를 확인하고, `WorkosCursorSessionToken=` 뒤의 값만 복사
4. **옵션 B - Application 탭**: Application → Cookies → `https://www.cursor.com`으로 이동, `WorkosCursorSessionToken` 쿠키를 찾아 값 복사 (쿠키 이름이 아닌 값)

> ⚠️ **보안 경고**: 세션 토큰을 비밀번호처럼 취급하세요. 절대 공개적으로 공유하거나 버전 관리에 커밋하지 마세요. 토큰은 Cursor 계정에 대한 전체 액세스 권한을 부여합니다.

### 예시 출력 (`--light` 버전)

<img alt="CLI Light" src="./.github/assets/cli-light.png" />

### 환경 변수

대용량 데이터셋이나 특수 요구사항이 있는 고급 사용자용:

| 변수 | 기본값 | 설명 |
|----------|---------|-------------|
| `TOKSCALE_CACHE_DIR` | `~/.cache/tokscale` | 가격 데이터를 위한 커스텀 캐시 디렉토리 |
| `TOKSCALE_NATIVE_TIMEOUT_MS` | `300000` (5분) | 네이티브 서브프로세스 처리 최대 시간 |
| `TOKSCALE_MAX_OUTPUT_BYTES` | `104857600` (100MB) | 네이티브 서브프로세스의 최대 출력 크기 |

```bash
# 예시: 커스텀 캐시 디렉토리 사용
TOKSCALE_CACHE_DIR=/path/to/cache tokscale

# 예시: 매우 큰 데이터셋에 대한 타임아웃 증가
TOKSCALE_NATIVE_TIMEOUT_MS=600000 tokscale graph --output data.json

# 예시: 수년간의 데이터를 가진 파워 유저를 위한 출력 제한 증가
TOKSCALE_MAX_OUTPUT_BYTES=104857600 tokscale --json > report.json
```

> **참고**: 이러한 제한은 중단 및 메모리 문제를 방지하기 위한 안전 조치입니다. 대부분의 사용자는 변경할 필요가 없습니다.

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
- **소스 필터링**: 플랫폼별 필터 (OpenCode, Claude, Codex, Cursor, Gemini)
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
cd packages/cli && bun src/cli.ts

# 또는 레거시 CLI 모드 사용
cd packages/cli && bun src/cli.ts --light
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

**참고**: 이 프로젝트는 **Bun**을 패키지 매니저 및 런타임으로 사용합니다. TUI는 OpenTUI의 네이티브 모듈 때문에 Bun이 필요합니다.

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
tokscale graph --opencode --claude

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

| 플랫폼 | 아키텍처 | 상태 |
|----------|--------------|--------|
| macOS | x86_64 | 지원 |
| macOS | aarch64 (Apple Silicon) | 지원 |
| Linux | x86_64 (glibc) | 지원 |
| Linux | aarch64 (glibc) | 지원 |
| Linux | x86_64 (musl) | 지원 |
| Linux | aarch64 (musl) | 지원 |
| Windows | x86_64 | 지원 |
| Windows | aarch64 | 지원 |

## 세션 데이터 보존

기본적으로 일부 AI 코딩 어시스턴트는 오래된 세션 파일을 자동으로 삭제합니다. 정확한 추적을 위해 사용 기록을 보존하려면 정리 기간을 비활성화하거나 연장하세요.

| 플랫폼 | 기본값 | 설정 파일 | 비활성화 설정 | 출처 |
|----------|---------|-------------|-------------------|--------|
| Claude Code | **⚠️ 30일** | `~/.claude/settings.json` | `"cleanupPeriodDays": 9999999999` | [문서](https://docs.anthropic.com/en/docs/claude-code/settings) |
| Gemini CLI | 비활성화됨 | `~/.gemini/settings.json` | `"sessionRetention.enabled": false` | [문서](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/session-management.md) |
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

정리를 활성화했다가 비활성화하려면 `~/.gemini/settings.json`에서 제거하거나 `enabled: false`로 설정:
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

위치: `~/.local/share/opencode/storage/message/{sessionId}/*.json`

각 메시지 파일 포함 내용:
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

위치: `~/.claude/projects/{projectPath}/*.jsonl`

어시스턴트 메시지의 사용량 데이터를 포함하는 JSONL 형식:
```json
{"type": "assistant", "message": {"model": "claude-sonnet-4-20250514", "usage": {"input_tokens": 1234, "output_tokens": 567, "cache_read_input_tokens": 890}}, "timestamp": "2024-01-01T00:00:00Z"}
```

### Codex CLI

위치: `~/.codex/sessions/*.jsonl`

`token_count` 이벤트가 있는 이벤트 기반 형식:
```json
{"type": "event_msg", "payload": {"type": "token_count", "info": {"last_token_usage": {"input_tokens": 1234, "output_tokens": 567}}}}
```

### Gemini CLI

위치: `~/.gemini/tmp/{projectHash}/chats/session-*.json`

메시지 배열을 포함하는 세션 파일:
```json
{
  "sessionId": "xxx",
  "messages": [
    {"type": "gemini", "model": "gemini-2.5-pro", "tokens": {"input": 1234, "output": 567, "cached": 890, "thoughts": 123}}
  ]
}
```

### Cursor IDE

위치: `~/.config/tokscale/cursor-cache/` (Cursor API를 통해 동기화)

Cursor 데이터는 세션 토큰을 사용하여 Cursor API에서 가져와 로컬에 캐시됩니다. 인증하려면 `tokscale cursor login`을 실행하세요. 설정 안내는 [Cursor IDE 명령어](#cursor-ide-명령어)를 참조하세요.

## 가격

Tokscale은 [LiteLLM의 가격 데이터베이스](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json)에서 실시간 가격을 가져옵니다.

**캐싱**: 가격 데이터는 1시간 TTL로 디스크에 캐시되어 빠른 시작을 보장합니다:
- LiteLLM 캐시: `~/.cache/tokscale/pricing-litellm.json`
- OpenRouter 캐시: `~/.cache/tokscale/pricing-openrouter.json` (증분 방식, 사용한 모델만 캐시)

가격 포함 항목:
- 입력 토큰
- 출력 토큰
- 캐시 읽기 토큰 (할인)
- 캐시 쓰기 토큰
- 추론 토큰 (o1과 같은 모델용)
- 구간별 가격 (200k 토큰 이상)

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
- 깜빡임 없는 터미널 UI 프레임워크 [OpenTUI](https://github.com/sst/opentui)
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
