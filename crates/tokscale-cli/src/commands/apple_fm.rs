//! Apple FoundationModels (on-device Apple Intelligence) summarizer backend.
//!
//! This module replaces the former `scripts/wiki-summarizer.py` Python backend
//! with native Rust FFI into Apple's FoundationModels via the vendored
//! `foundation-models-c` C-ABI package.
//!
//! The real FFI implementation is gated behind `cfg(all(target_os = "macos",
//! feature = "apple-fm"))`. On every other target/feature combination a stub
//! `summarize` returning `None` is compiled so the caller transparently falls
//! back to the Rust heuristic ([`heuristic_classify`]), which is always
//! available and cross-platform.
//!
//! Availability gate: [`summarize`] returns `None` (never errors) when Apple
//! Intelligence is unavailable, so the caller degrades to the heuristic.

/// Input metadata for one coding session to be summarized.
///
/// Some fields (`client`, `first_user_message`, `message_count`) feed only the
/// FM prompt, so they are unread on the heuristic-only (feature-off / non-macOS)
/// build path.
#[cfg_attr(not(all(target_os = "macos", feature = "apple-fm")), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct SessionInput {
    pub session_id: String,
    pub client: String,
    pub workspace: String,
    pub first_user_message: Option<String>,
    pub models_used: Vec<String>,
    pub total_tokens: i64,
    pub duration_minutes: i64,
    pub message_count: i64,
}

/// Structured summary produced for one session.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub session_id: String,
    pub title: String,
    pub task_category: String,
    pub description: String,
    pub complexity: String,
    /// Provenance of THIS summary: `Some("apple-fm-on-device")` when produced by
    /// Apple FM, `None` when it came from the heuristic (including per-session
    /// fallbacks). Carried per-summary so heuristic results are never recorded
    /// as Apple-FM-generated.
    pub fm_version: Option<String>,
}

/// Allowed task categories. Anything else is coerced to `"other"`.
/// Only consumed by the FM validation path (feature-gated).
#[cfg_attr(not(all(target_os = "macos", feature = "apple-fm")), allow(dead_code))]
pub const VALID_CATEGORIES: &[&str] = &[
    "feature", "bugfix", "refactor", "research", "debug", "review", "docs", "config", "other",
];

/// Allowed complexity levels. Anything else is coerced to `"moderate"`.
/// Only consumed by the FM validation path (feature-gated).
#[cfg_attr(not(all(target_os = "macos", feature = "apple-fm")), allow(dead_code))]
pub const VALID_COMPLEXITIES: &[&str] = &["trivial", "moderate", "complex"];

/// Deterministic, cross-platform fallback classifier.
///
/// Direct port of the former Python `fallback_classify`, with identical
/// thresholds:
/// - complexity: `total_tokens > 200_000 || duration_minutes > 120` => `complex`;
///   else `total_tokens > 50_000 || duration_minutes > 30` => `moderate`;
///   else `trivial`.
/// - project name: the path component after the last `/` of the workspace, or
///   `"unknown"` when the workspace is empty.
/// - title: `Work on {project_name}`; category: `other`;
///   description: `Session in {project_name} using {models joined ", "}.`
///   (models default to `unknown` when none are recorded).
pub fn heuristic_classify(session: &SessionInput) -> SessionSummary {
    let complexity = if session.total_tokens > 200_000 || session.duration_minutes > 120 {
        "complex"
    } else if session.total_tokens > 50_000 || session.duration_minutes > 30 {
        "moderate"
    } else {
        "trivial"
    };

    let project_name = if session.workspace.is_empty() {
        "unknown".to_string()
    } else {
        session
            .workspace
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_string()
    };

    let models = if session.models_used.is_empty() {
        "unknown".to_string()
    } else {
        session.models_used.join(", ")
    };

    SessionSummary {
        session_id: session.session_id.clone(),
        title: format!("Work on {project_name}"),
        task_category: "other".to_string(),
        description: format!("Session in {project_name} using {models}."),
        complexity: complexity.to_string(),
        fm_version: None,
    }
}

#[cfg(all(target_os = "macos", feature = "apple-fm"))]
mod imp {
    use super::{heuristic_classify, SessionInput, SessionSummary};
    use super::{VALID_CATEGORIES, VALID_COMPLEXITIES};
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Upper bound on a single on-device generation. A short classification
    /// completes in seconds; this only guards against a callback that never
    /// fires (which would otherwise block the calling thread forever). On
    /// timeout the session falls back to the heuristic.
    const FM_GENERATION_TIMEOUT: Duration = Duration::from_secs(60);

    /// Verbatim system instructions for the classifier (matches the former
    /// Python backend exactly).
    const SYSTEM_INSTRUCTIONS: &str = "You are a coding session classifier. Given metadata about an AI coding session, produce a structured summary.\n\nRules:\n- title: 3-8 word description of what was done (imperative mood, e.g. \"Add JWT auth middleware\")\n- task_category: exactly one of: feature, bugfix, refactor, research, debug, review, docs, config, other\n- description: 1-2 sentences explaining what happened in the session\n- complexity: exactly one of: trivial, moderate, complex\n\nBase your classification on:\n- The first user message (primary signal)\n- The workspace name (project context)\n- Token count and duration (complexity signal)\n- Models used (opus = likely complex, haiku = likely trivial)\n\nRespond ONLY with valid JSON matching the schema.";

    /// JSON-schema string passed to `RespondWithSchemaFromJSON`. A simple
    /// object-with-properties; the result is re-validated in Rust regardless.
    const SCHEMA_JSON: &str = r#"{
  "type": "object",
  "properties": {
    "title": { "type": "string" },
    "task_category": {
      "type": "string",
      "enum": ["feature", "bugfix", "refactor", "research", "debug", "review", "docs", "config", "other"]
    },
    "description": { "type": "string" },
    "complexity": {
      "type": "string",
      "enum": ["trivial", "moderate", "complex"]
    }
  },
  "required": ["title", "task_category", "description", "complexity"]
}"#;

    // Opaque FoundationModels handles. All are `const void*` in the C ABI.
    type FMRef = *const c_void;

    /// Callback signature: `void (*)(int status, FMGeneratedContentRef content, void* userInfo)`.
    type StructuredCallback = extern "C" fn(status: c_int, content: FMRef, user_info: *mut c_void);

    #[allow(non_snake_case)]
    extern "C" {
        fn FMSystemLanguageModelGetDefault() -> FMRef;
        fn FMSystemLanguageModelIsAvailable(model: FMRef, unavailable_reason: *mut c_int) -> bool;
        fn FMLanguageModelSessionCreateFromSystemLanguageModel(
            model: FMRef,
            instructions: *const c_char,
            tools: *mut FMRef,
            tool_count: c_int,
        ) -> FMRef;
        fn FMComposedPromptInitialize() -> FMRef;
        fn FMComposedPromptAddText(composed_prompt: FMRef, text: *const c_char);
        fn FMLanguageModelSessionRespondWithSchemaFromJSON(
            session: FMRef,
            composed_prompt: FMRef,
            schema_json: *const c_char,
            options_json: *const c_char,
            user_info: *mut c_void,
            callback: StructuredCallback,
        ) -> FMRef;
        fn FMGeneratedContentGetJSONString(content: FMRef) -> *mut c_char;
        fn FMRelease(object: FMRef);
        fn FMFreeString(s: *mut c_char);
    }

    /// What the background callback ships back to the blocked calling thread:
    /// `Ok(json)` on success, `Err(status)` on failure.
    type CallbackResult = Result<String, c_int>;

    /// Heap-allocated channel sender handed to the C callback as `userInfo`.
    struct CallbackBox {
        tx: mpsc::Sender<CallbackResult>,
    }

    /// The structured-response callback. Invoked on a BACKGROUND thread by the
    /// Swift bridge. Copies the JSON out of the generated content and signals
    /// the waiting thread via the channel.
    extern "C" fn structured_callback(status: c_int, content: FMRef, user_info: *mut c_void) {
        // Reconstruct the boxed sender. We own it now and drop it at end of scope.
        if user_info.is_null() {
            return;
        }
        let cb: Box<CallbackBox> = unsafe { Box::from_raw(user_info as *mut CallbackBox) };

        let result: CallbackResult = if status != 0 || content.is_null() {
            Err(status)
        } else {
            // SAFETY: content is non-null per the check above; the returned
            // string is malloc'd and must be freed via FMFreeString.
            let json_ptr = unsafe { FMGeneratedContentGetJSONString(content) };
            if json_ptr.is_null() {
                Err(status)
            } else {
                let json = unsafe { CStr::from_ptr(json_ptr) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { FMFreeString(json_ptr) };
                Ok(json)
            }
        };

        // Best-effort send; if the receiver is gone there is nothing to do.
        let _ = cb.tx.send(result);
    }

    /// Build the per-session prompt text (matches the former Python `build_prompt`).
    fn build_prompt(input: &SessionInput) -> String {
        let workspace = if input.workspace.is_empty() {
            "unknown"
        } else {
            input.workspace.as_str()
        };
        let client = if input.client.is_empty() {
            "unknown"
        } else {
            input.client.as_str()
        };
        let models = input.models_used.join(", ");

        let mut s = format!(
            "Workspace: {workspace}\nClient: {client}\nModels: {models}\nTotal tokens: {}\nDuration: {} minutes\nMessages: {}",
            input.total_tokens, input.duration_minutes, input.message_count
        );

        match &input.first_user_message {
            Some(msg) if !msg.is_empty() => {
                s.push_str("\n\nFirst user message:\n");
                s.push_str(msg);
            }
            _ => {
                s.push_str("\n\nNo user message content available.");
            }
        }
        s
    }

    /// Coerce parsed category/complexity to the allowed sets.
    fn normalize_category(raw: &str) -> String {
        if VALID_CATEGORIES.contains(&raw) {
            raw.to_string()
        } else {
            "other".to_string()
        }
    }
    fn normalize_complexity(raw: &str) -> String {
        if VALID_COMPLEXITIES.contains(&raw) {
            raw.to_string()
        } else {
            "moderate".to_string()
        }
    }

    /// Parse the FM-returned JSON into a [`SessionSummary`], coercing invalid
    /// enum values. Returns `None` if the JSON is unusable.
    fn parse_summary(session_id: &str, json: &str) -> Option<SessionSummary> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let title = value
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled session")
            .to_string();
        let task_category = normalize_category(
            value
                .get("task_category")
                .and_then(|v| v.as_str())
                .unwrap_or("other"),
        );
        let description = value
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let complexity = normalize_complexity(
            value
                .get("complexity")
                .and_then(|v| v.as_str())
                .unwrap_or("moderate"),
        );
        Some(SessionSummary {
            session_id: session_id.to_string(),
            title,
            task_category,
            description,
            complexity,
            fm_version: Some("apple-fm-on-device".to_string()),
        })
    }

    /// Run a single structured generation for `input`, blocking the calling
    /// thread until the background callback fires. Returns the parsed summary,
    /// or `None` on any error (caller falls back to the heuristic).
    ///
    /// A FRESH `LanguageModelSession` is created per input: the session is
    /// stateful (it accumulates a transcript), so reusing one across sessions
    /// would condition later summaries on earlier prompts/responses — and a
    /// timed-out generation could leave a shared session busy. This mirrors the
    /// former Python backend, which built a new session inside its loop.
    fn respond_one(
        model: FMRef,
        instructions: &CStr,
        schema: &CStr,
        input: &SessionInput,
    ) -> Option<SessionSummary> {
        let session_ref = unsafe {
            FMLanguageModelSessionCreateFromSystemLanguageModel(
                model,
                instructions.as_ptr(),
                std::ptr::null_mut(),
                0,
            )
        };
        if session_ref.is_null() {
            return None;
        }

        // Build the prompt CString BEFORE allocating the composed-prompt handle,
        // so an unexpected NUL byte cannot leak an allocated FM handle.
        let prompt_text = match CString::new(build_prompt(input)) {
            Ok(c) => c,
            Err(_) => {
                unsafe { FMRelease(session_ref) };
                return None;
            }
        };
        let prompt_ref = unsafe { FMComposedPromptInitialize() };
        if prompt_ref.is_null() {
            unsafe { FMRelease(session_ref) };
            return None;
        }
        unsafe { FMComposedPromptAddText(prompt_ref, prompt_text.as_ptr()) };

        let (tx, rx) = mpsc::channel::<CallbackResult>();
        let cb_box = Box::new(CallbackBox { tx });
        let user_info = Box::into_raw(cb_box) as *mut c_void;

        let task_ref = unsafe {
            FMLanguageModelSessionRespondWithSchemaFromJSON(
                session_ref,
                prompt_ref,
                schema.as_ptr(),
                std::ptr::null(),
                user_info,
                structured_callback,
            )
        };

        // Block on the background callback (bounded). The callback reclaims
        // `user_info`; on timeout the box is intentionally leaked rather than
        // risk a use-after-free if the callback fires later.
        let received = rx.recv_timeout(FM_GENERATION_TIMEOUT);

        // Release the task handle, composed prompt, and this input's session.
        if !task_ref.is_null() {
            unsafe { FMRelease(task_ref) };
        }
        unsafe { FMRelease(prompt_ref) };
        unsafe { FMRelease(session_ref) };

        match received {
            Ok(Ok(json)) => parse_summary(&input.session_id, &json),
            _ => None,
        }
    }

    /// Real FFI implementation. See module docs and [`super::summarize`].
    pub fn summarize(sessions: &[SessionInput]) -> Option<Vec<SessionSummary>> {
        if sessions.is_empty() {
            return Some(Vec::new());
        }

        // 1) Default model + availability gate. NEVER generate if unavailable.
        let model = unsafe { FMSystemLanguageModelGetDefault() };
        if model.is_null() {
            return None;
        }
        let available = unsafe { FMSystemLanguageModelIsAvailable(model, std::ptr::null_mut()) };
        if !available {
            unsafe { FMRelease(model) };
            return None;
        }

        // 2) Prepare the shared instructions + output schema once. respond_one
        //    creates a FRESH session per input from these (see its docs).
        let instructions = match CString::new(SYSTEM_INSTRUCTIONS) {
            Ok(c) => c,
            Err(_) => {
                unsafe { FMRelease(model) };
                return None;
            }
        };
        let schema = match CString::new(SCHEMA_JSON) {
            Ok(c) => c,
            Err(_) => {
                unsafe { FMRelease(model) };
                return None;
            }
        };

        // 3) One structured generation per session; per-session errors fall
        //    back to the heuristic for that single session.
        let mut results = Vec::with_capacity(sessions.len());
        for input in sessions {
            match respond_one(model, instructions.as_c_str(), schema.as_c_str(), input) {
                Some(summary) => results.push(summary),
                None => results.push(heuristic_classify(input)),
            }
        }

        unsafe { FMRelease(model) };

        Some(results)
    }
}

/// Summarize sessions using Apple's on-device FoundationModels.
///
/// Returns:
/// - `Some(results)` when the model is available and generation ran (per-session
///   failures are individually backfilled with [`heuristic_classify`]).
/// - `None` when Apple Intelligence is unavailable, the feature is off, or the
///   target is not macOS. The caller must then apply the heuristic to all
///   sessions. This function never errors.
#[cfg(all(target_os = "macos", feature = "apple-fm"))]
pub fn summarize(sessions: &[SessionInput]) -> Option<Vec<SessionSummary>> {
    imp::summarize(sessions)
}

/// Stub used when the `apple-fm` feature is off or the target is not macOS.
/// Always returns `None` so the caller falls back to the heuristic.
#[cfg(not(all(target_os = "macos", feature = "apple-fm")))]
pub fn summarize(_sessions: &[SessionInput]) -> Option<Vec<SessionSummary>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        total_tokens: i64,
        duration_minutes: i64,
        workspace: &str,
        models: &[&str],
    ) -> SessionInput {
        SessionInput {
            session_id: "ses_test".to_string(),
            client: "opencode".to_string(),
            workspace: workspace.to_string(),
            first_user_message: None,
            models_used: models.iter().map(|s| s.to_string()).collect(),
            total_tokens,
            duration_minutes,
            message_count: 1,
        }
    }

    #[test]
    fn complexity_complex_by_tokens() {
        let s = heuristic_classify(&input(200_001, 0, "/x/proj", &["opus"]));
        assert_eq!(s.complexity, "complex");
    }

    #[test]
    fn complexity_complex_by_duration() {
        let s = heuristic_classify(&input(0, 121, "/x/proj", &["opus"]));
        assert_eq!(s.complexity, "complex");
    }

    #[test]
    fn complexity_moderate_by_tokens() {
        let s = heuristic_classify(&input(50_001, 0, "/x/proj", &["sonnet"]));
        assert_eq!(s.complexity, "moderate");
    }

    #[test]
    fn complexity_moderate_by_duration() {
        let s = heuristic_classify(&input(0, 31, "/x/proj", &["sonnet"]));
        assert_eq!(s.complexity, "moderate");
    }

    #[test]
    fn complexity_trivial() {
        let s = heuristic_classify(&input(50_000, 30, "/x/proj", &["haiku"]));
        assert_eq!(s.complexity, "trivial");
    }

    #[test]
    fn complexity_boundaries_are_exclusive() {
        // Exactly at the thresholds => the lower tier (strictly-greater compares).
        assert_eq!(
            heuristic_classify(&input(200_000, 120, "/x/p", &[])).complexity,
            "moderate"
        );
        assert_eq!(
            heuristic_classify(&input(50_000, 30, "/x/p", &[])).complexity,
            "trivial"
        );
    }

    #[test]
    fn project_name_and_title_from_workspace() {
        let s = heuristic_classify(&input(0, 0, "/Users/x/tokscale", &["claude-opus-4"]));
        assert_eq!(s.title, "Work on tokscale");
        assert_eq!(s.task_category, "other");
        assert_eq!(s.description, "Session in tokscale using claude-opus-4.");
    }

    #[test]
    fn project_name_unknown_when_empty_workspace() {
        let s = heuristic_classify(&input(0, 0, "", &[]));
        assert_eq!(s.title, "Work on unknown");
        assert_eq!(s.description, "Session in unknown using unknown.");
    }

    #[test]
    fn description_joins_multiple_models() {
        let s = heuristic_classify(&input(0, 0, "/a/b/myrepo", &["opus", "haiku"]));
        assert_eq!(s.title, "Work on myrepo");
        assert_eq!(s.description, "Session in myrepo using opus, haiku.");
    }

    #[test]
    fn stub_or_gate_returns_some_or_none_without_panicking() {
        // On non-macOS / feature-off this returns None; on macOS+feature it may
        // return None (unavailable) or Some. Either way it must not panic.
        let _ = summarize(&[input(1, 1, "/x/p", &["m"])]);
    }
}
