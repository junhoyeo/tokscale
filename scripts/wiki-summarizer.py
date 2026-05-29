#!/usr/bin/env python3
"""
tokscale wiki summarizer — uses Apple Foundation Models (on-device) to
classify and summarize coding sessions.

Protocol:
  stdin:  JSON array of session objects to summarize
  stdout: JSON array of summary results

Each input object:
  {
    "session_id": "ses_abc123",
    "client": "opencode",
    "workspace": "/Users/x/project",
    "first_user_message": "Add JWT auth middleware...",
    "models_used": ["claude-opus-4"],
    "total_tokens": 145000,
    "duration_minutes": 120,
    "message_count": 47
  }

Each output object:
  {
    "session_id": "ses_abc123",
    "title": "Implement JWT auth middleware",
    "task_category": "feature",
    "description": "Added JWT-based authentication...",
    "complexity": "complex",
    "fm_version": "apple-fm-on-device"
  }
"""

import sys
import json
import asyncio

try:
    import apple_fm_sdk as fm

    FM_AVAILABLE = True
except ImportError:
    FM_AVAILABLE = False


SYSTEM_INSTRUCTIONS = """You are a coding session classifier. Given metadata about an AI coding session, produce a structured summary.

Rules:
- title: 3-8 word description of what was done (imperative mood, e.g. "Add JWT auth middleware")
- task_category: exactly one of: feature, bugfix, refactor, research, debug, review, docs, config, other
- description: 1-2 sentences explaining what happened in the session
- complexity: exactly one of: trivial, moderate, complex

Base your classification on:
- The first user message (primary signal)
- The workspace name (project context)
- Token count and duration (complexity signal)
- Models used (opus = likely complex, haiku = likely trivial)

Respond ONLY with valid JSON matching the schema."""


def build_prompt(session: dict) -> str:
    parts = []
    parts.append(f"Workspace: {session.get('workspace', 'unknown')}")
    parts.append(f"Client: {session.get('client', 'unknown')}")
    parts.append(f"Models: {', '.join(session.get('models_used', []))}")
    parts.append(f"Total tokens: {session.get('total_tokens', 0):,}")
    parts.append(f"Duration: {session.get('duration_minutes', 0)} minutes")
    parts.append(f"Messages: {session.get('message_count', 0)}")

    first_msg = session.get("first_user_message")
    if first_msg:
        parts.append(f"\nFirst user message:\n{first_msg}")
    else:
        parts.append("\nNo user message content available.")

    parts.append(
        '\nRespond with JSON: {"title": "...", "task_category": "...", "description": "...", "complexity": "..."}'
    )

    return "\n".join(parts)


VALID_CATEGORIES = {"feature", "bugfix", "refactor", "research", "debug", "review", "docs", "config", "other"}
VALID_COMPLEXITIES = {"trivial", "moderate", "complex"}


def fallback_classify(session: dict) -> dict:
    total_tokens = session.get("total_tokens", 0)
    duration = session.get("duration_minutes", 0)

    if total_tokens > 200000 or duration > 120:
        complexity = "complex"
    elif total_tokens > 50000 or duration > 30:
        complexity = "moderate"
    else:
        complexity = "trivial"

    workspace = session.get("workspace", "")
    project_name = workspace.rsplit("/", 1)[-1] if workspace else "unknown"

    return {
        "session_id": session["session_id"],
        "title": f"Work on {project_name}",
        "task_category": "other",
        "description": f"Session in {project_name} using {', '.join(session.get('models_used', ['unknown']))}.",
        "complexity": complexity,
        "fm_version": None,
    }


async def summarize_with_fm(sessions: list[dict]) -> list[dict]:
    model = fm.SystemLanguageModel()
    is_available, reason = model.is_available()

    if not is_available:
        print(f"FM not available: {reason}", file=sys.stderr)
        return [fallback_classify(s) for s in sessions]

    results = []
    for session in sessions:
        try:
            fm_session = fm.LanguageModelSession(instructions=SYSTEM_INSTRUCTIONS)
            prompt = build_prompt(session)
            response = await fm_session.respond(prompt)
            response_text = str(response)

            parsed = json.loads(response_text)
            task_category = parsed.get("task_category", "other")
            if task_category not in VALID_CATEGORIES:
                task_category = "other"
            complexity = parsed.get("complexity", "moderate")
            if complexity not in VALID_COMPLEXITIES:
                complexity = "moderate"
            results.append(
                {
                    "session_id": session["session_id"],
                    "title": parsed.get("title", "Untitled session"),
                    "task_category": task_category,
                    "description": parsed.get("description", ""),
                    "complexity": complexity,
                    "fm_version": "apple-fm-on-device",
                }
            )
        except (json.JSONDecodeError, Exception) as e:
            session_id = session.get("session_id", "unknown")
            print(f"FM error for {session_id}: {e}", file=sys.stderr)
            results.append(fallback_classify(session))

    return results


def main():
    input_data = json.loads(sys.stdin.read())

    if not isinstance(input_data, list):
        input_data = [input_data]

    if not input_data:
        json.dump([], sys.stdout)
        return

    if FM_AVAILABLE:
        results = asyncio.run(summarize_with_fm(input_data))
    else:
        print("apple-fm-sdk not installed, using fallback classification", file=sys.stderr)
        results = [fallback_classify(s) for s in input_data]

    json.dump(results, sys.stdout, ensure_ascii=False)


if __name__ == "__main__":
    main()
