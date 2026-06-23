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
//! ## Why `dlopen` instead of linking
//!
//! `FoundationModels.framework` only exists on macOS 26+, and the Swift runtime
//! the shim drags in (`libswiftSynchronization`, macOS 15+, etc.) does too. If
//! the `tokscale` binary hard-linked any of them it would fail to `dyld`-load on
//! older macOS — a crash-on-launch for EVERY command, not a feature fallback.
//! And `import FoundationModels` autolinks the framework as a non-weak load
//! command, so a `-weak_framework` flag can't reliably flip it.
//!
//! So the binary links nothing FM/Swift (verifiable: `otool -L tokscale` shows
//! no FoundationModels and no `libswift*`). The vendored shim is built as a
//! DYNAMIC `libFoundationModels.dylib` (see `build.rs`) staged next to the
//! binary, and this module `dlopen`s it lazily — only on macOS 26+, where all
//! its dependencies exist. On older macOS the `dlopen` simply fails and the
//! caller degrades to the heuristic. This keeps a SINGLE arm64 binary safe to
//! ship to every Apple Silicon Mac via npm, regardless of their macOS version.
//!
//! Availability gate: [`summarize`] returns `None` (never errors) when the
//! dylib can't be loaded (old macOS / missing file) OR Apple Intelligence is
//! unavailable, so the caller degrades to the heuristic.
//!
//! Smoke-testing note: the vendored `fm-c-example` binary's streaming path
//! (`FMLanguageModelSessionStreamResponse`) hard-segfaults (EXC_BAD_ACCESS in
//! `objc_retain`) on macOS 26.2, so it is NOT a valid liveness check for "is FM
//! working on this box". This module uses only the non-streaming
//! `FMLanguageModelSessionRespondWithSchema` path (a PROGRAMMATIC
//! GenerationSchema built via [`imp::build_schema`], NOT the JSON-Schema-string
//! `...FromJSON` variant) and is unaffected. For end-to-end verification use the
//! `#[ignore]`d live test in this module (`live_summarize_smoke`), not the
//! streaming example.

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
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::OnceLock;
    use std::time::Duration;

    /// Upper bound on a single on-device generation. A short classification
    /// completes in seconds; this only guards against a callback that never
    /// fires (which would otherwise block the calling thread forever). On
    /// timeout the session falls back to the heuristic.
    const FM_GENERATION_TIMEOUT: Duration = Duration::from_secs(60);

    /// Upper bound on the first-user-message text appended to the prompt. The
    /// on-device model has a small context window, so a large pasted message
    /// (stack trace, file dump, multi-KB prompt) is truncated here. Larger than
    /// the CLI backend's 200-char cap since the FM prompt carries only one
    /// session at a time.
    const MAX_FIRST_USER_MESSAGE_CHARS: usize = 1000;

    /// First macOS major version that ships `FoundationModels.framework`.
    const FM_MIN_MACOS_MAJOR: u32 = 26;

    /// Verbatim system instructions for the classifier (matches the former
    /// Python backend exactly).
    const SYSTEM_INSTRUCTIONS: &str = "You are a coding session classifier. Given metadata about an AI coding session, produce a structured summary.\n\nRules:\n- title: 3-8 word description of what was done (imperative mood, e.g. \"Add JWT auth middleware\")\n- task_category: exactly one of: feature, bugfix, refactor, research, debug, review, docs, config, other\n- description: 1-2 sentences explaining what happened in the session\n- complexity: exactly one of: trivial, moderate, complex\n\nBase your classification on:\n- The first user message (primary signal)\n- The workspace name (project context)\n- Token count and duration (complexity signal)\n- Models used (opus = likely complex, haiku = likely trivial)\n\nRespond ONLY with valid JSON matching the schema.";

    // Opaque FoundationModels handles. All are `const void*` in the C ABI.
    type FMRef = *const c_void;

    /// Callback signature: `void (*)(int status, FMGeneratedContentRef content, void* userInfo)`.
    type StructuredCallback = extern "C" fn(status: c_int, content: FMRef, user_info: *mut c_void);

    // --- dl* / sysctl: libSystem symbols, ALWAYS present, no FM/Swift linkage.
    extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *const c_char;
        fn sysctlbyname(
            name: *const c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> c_int;
    }
    const RTLD_NOW: c_int = 2;
    const RTLD_LOCAL: c_int = 4;

    // --- Resolved FoundationModels C-ABI entry points (loaded via dlsym). The
    // signatures mirror the vendored `foundation-models-c` header exactly.
    type FnGetDefault = unsafe extern "C" fn() -> FMRef;
    type FnIsAvailable = unsafe extern "C" fn(FMRef, *mut c_int) -> bool;
    type FnSessionCreate = unsafe extern "C" fn(FMRef, *const c_char, *mut FMRef, c_int) -> FMRef;
    type FnPromptInit = unsafe extern "C" fn() -> FMRef;
    type FnPromptAddText = unsafe extern "C" fn(FMRef, *const c_char);
    type FnSchemaCreate = unsafe extern "C" fn(*const c_char, *const c_char) -> FMRef;
    type FnPropertyCreate =
        unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, bool) -> FMRef;
    type FnPropertyAddAnyOf = unsafe extern "C" fn(FMRef, *const *const c_char, c_int, bool);
    type FnSchemaAddProperty = unsafe extern "C" fn(FMRef, FMRef);
    type FnRespondWithSchema = unsafe extern "C" fn(
        FMRef,
        FMRef,
        FMRef,
        *const c_char,
        *mut c_void,
        StructuredCallback,
    ) -> FMRef;
    type FnContentGetJSON = unsafe extern "C" fn(FMRef) -> *mut c_char;
    type FnRelease = unsafe extern "C" fn(FMRef);
    type FnFreeString = unsafe extern "C" fn(*mut c_char);

    /// The resolved FoundationModels API plus the (never-closed) dlopen handle.
    ///
    /// `FMGenerationSchemaCreate` / `...PropertyCreate` return a +1-retained ref
    /// (`Unmanaged.passRetained`), so each must be `release`d. The builder's
    /// `addProperty` copies the property into a Swift array (it holds its own
    /// strong reference), so a property ref may be released immediately after
    /// `schema_add_property`. The structured-response callback receives a
    /// `content` handle the shim hands over with a +1 retain on EVERY invocation
    /// (success AND error/cancel); `content_get_json` only borrows it, so the
    /// callback owns that +1 and must `release(content)` exactly once on every
    /// path or one generated-content wrapper leaks per generation.
    #[allow(dead_code)]
    struct Api {
        /// Kept alive for the process lifetime; the dylib is never `dlclose`d.
        handle: *mut c_void,
        get_default: FnGetDefault,
        is_available: FnIsAvailable,
        session_create: FnSessionCreate,
        prompt_init: FnPromptInit,
        prompt_add_text: FnPromptAddText,
        schema_create: FnSchemaCreate,
        property_create: FnPropertyCreate,
        property_add_anyof: FnPropertyAddAnyOf,
        schema_add_property: FnSchemaAddProperty,
        respond_with_schema: FnRespondWithSchema,
        content_get_json: FnContentGetJSON,
        release: FnRelease,
        free_string: FnFreeString,
    }

    // `Api` holds only function pointers and an opaque handle; the function
    // pointers are immutable after load and safe to call from any thread (the
    // background structured callback reads them via `api()`).
    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}

    impl Api {
        /// dlsym every entry point off `handle`. Returns `None` if any symbol is
        /// missing (treated as "FM unavailable" -> heuristic).
        unsafe fn load(handle: *mut c_void) -> Option<Api> {
            // dlsym + transmute one symbol. `transmute_copy` because `T` is a
            // (pointer-sized) fn-pointer type and plain `transmute` can't prove
            // size equality for a generic.
            unsafe fn sym<T>(handle: *mut c_void, name: &[u8]) -> Option<T> {
                debug_assert_eq!(
                    name.last(),
                    Some(&0u8),
                    "symbol name must be NUL-terminated"
                );
                let p = dlsym(handle, name.as_ptr() as *const c_char);
                if p.is_null() {
                    None
                } else {
                    Some(std::mem::transmute_copy::<*mut c_void, T>(&p))
                }
            }

            Some(Api {
                handle,
                get_default: sym(handle, b"FMSystemLanguageModelGetDefault\0")?,
                is_available: sym(handle, b"FMSystemLanguageModelIsAvailable\0")?,
                session_create: sym(
                    handle,
                    b"FMLanguageModelSessionCreateFromSystemLanguageModel\0",
                )?,
                prompt_init: sym(handle, b"FMComposedPromptInitialize\0")?,
                prompt_add_text: sym(handle, b"FMComposedPromptAddText\0")?,
                schema_create: sym(handle, b"FMGenerationSchemaCreate\0")?,
                property_create: sym(handle, b"FMGenerationSchemaPropertyCreate\0")?,
                property_add_anyof: sym(handle, b"FMGenerationSchemaPropertyAddAnyOfGuide\0")?,
                schema_add_property: sym(handle, b"FMGenerationSchemaAddProperty\0")?,
                respond_with_schema: sym(handle, b"FMLanguageModelSessionRespondWithSchema\0")?,
                content_get_json: sym(handle, b"FMGeneratedContentGetJSONString\0")?,
                release: sym(handle, b"FMRelease\0")?,
                free_string: sym(handle, b"FMFreeString\0")?,
            })
        }
    }

    /// Read the macOS major version via `sysctl kern.osproductversion`
    /// (e.g. `"26.1"` -> `26`). `None` if it can't be determined.
    fn macos_major() -> Option<u32> {
        let name = b"kern.osproductversion\0";
        let mut size: usize = 0;
        // Probe the buffer size.
        let rc = unsafe {
            sysctlbyname(
                name.as_ptr() as *const c_char,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size];
        let rc = unsafe {
            sysctlbyname(
                name.as_ptr() as *const c_char,
                buf.as_mut_ptr() as *mut c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            return None;
        }
        let s = CStr::from_bytes_until_nul(&buf).ok()?.to_str().ok()?;
        s.split('.').next()?.parse::<u32>().ok()
    }

    /// Candidate `libFoundationModels.dylib` locations, in priority order:
    /// 1. next to the running binary (npm package layout, `cargo run`),
    /// 2. the absolute OUT_DIR copy baked in at build time (`cargo test`, where
    ///    the test harness binary lives in `target/<profile>/deps`).
    fn candidate_paths() -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                v.push(dir.join("libFoundationModels.dylib"));
            }
        }
        if let Some(p) = option_env!("TOKSCALE_FM_DYLIB") {
            v.push(PathBuf::from(p));
        }
        v
    }

    /// Load (once) the FoundationModels dylib and resolve its entry points.
    ///
    /// Returns `None` — caller falls back to the heuristic — when:
    /// - the OS is older than macOS 26 (no `FoundationModels.framework`), or
    /// - the dylib isn't found / can't be loaded (e.g. dependencies absent), or
    /// - an expected symbol is missing.
    fn load_api() -> Option<Api> {
        // Set `TOKSCALE_FM_DEBUG=1` to trace why apple-fm did/didn't engage
        // (OS gate, which dylib path loaded, dlopen errors, symbol resolution).
        let debug = std::env::var_os("TOKSCALE_FM_DEBUG").is_some();

        // Fast OS gate. The dylib's transitive deps (FoundationModels.framework
        // + macOS-26 Swift runtime) only exist on macOS 26+, so on older systems
        // the dlopen below would fail anyway; this documents the contract and
        // avoids a doomed load attempt.
        let major = macos_major();
        if debug {
            eprintln!("  apple-fm[debug]: macos_major={major:?}");
        }
        if let Some(major) = major {
            if major < FM_MIN_MACOS_MAJOR {
                if debug {
                    eprintln!(
                        "  apple-fm[debug]: OS gate {major} < {FM_MIN_MACOS_MAJOR} -> heuristic"
                    );
                }
                return None;
            }
        }

        for path in candidate_paths() {
            let exists = path.exists();
            let c = match CString::new(path.as_os_str().as_bytes()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let handle = unsafe { dlopen(c.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
            if handle.is_null() {
                if debug {
                    let err = unsafe { dlerror() };
                    let msg = if err.is_null() {
                        "(no dlerror)".to_string()
                    } else {
                        unsafe { CStr::from_ptr(err) }
                            .to_string_lossy()
                            .into_owned()
                    };
                    eprintln!(
                        "  apple-fm[debug]: dlopen failed (exists={exists}) {} :: {msg}",
                        path.display()
                    );
                }
                continue;
            }
            if debug {
                eprintln!("  apple-fm[debug]: dlopen ok {}", path.display());
            }
            // Leave the handle open for the process lifetime on success; on a
            // (very unexpected) missing symbol, move on to the next candidate.
            if let Some(api) = unsafe { Api::load(handle) } {
                return Some(api);
            }
            if debug {
                eprintln!(
                    "  apple-fm[debug]: symbol resolution failed for {}",
                    path.display()
                );
            }
        }
        if debug {
            eprintln!("  apple-fm[debug]: no usable FoundationModels dylib -> heuristic");
        }
        None
    }

    /// Process-wide resolved API, or `None` if FM is unavailable on this box.
    fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(load_api).as_ref()
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

        // The callback can only fire after a successful `respond_with_schema`
        // call, which required `api()` to be `Some`; so this lookup never fails
        // in practice, but we degrade to `Err(status)` if it somehow does.
        let api = api();

        let result: CallbackResult = match api {
            Some(api) if status == 0 && !content.is_null() => {
                // SAFETY: content is non-null; the returned string is malloc'd
                // and must be freed via `free_string`.
                let json_ptr = unsafe { (api.content_get_json)(content) };
                if json_ptr.is_null() {
                    Err(status)
                } else {
                    let json = unsafe { CStr::from_ptr(json_ptr) }
                        .to_string_lossy()
                        .into_owned();
                    unsafe { (api.free_string)(json_ptr) };
                    Ok(json)
                }
            }
            _ => Err(status),
        };

        // The shim hands us a +1-retained `content` on EVERY callback path
        // (success and error/cancel) and `content_get_json` only borrows it, so
        // we own that retain and must release it here exactly once or one
        // generated-content wrapper leaks per generation.
        if !content.is_null() {
            if let Some(api) = api {
                unsafe { (api.release)(content) };
            }
        }

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
                // Cap the (possibly multi-KB) pasted message: on-device FM has a
                // small context window, so an oversized prompt risks truncation,
                // refusal, or latency. `chars().take` is char-boundary-safe.
                let capped: String = msg.chars().take(MAX_FIRST_USER_MESSAGE_CHARS).collect();
                s.push_str(&capped);
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

    /// Build the programmatic `SessionSummary` GenerationSchema once, enforcing
    /// the category/complexity enums on-device via `anyOf` guides. Returns a
    /// +1-retained schema ref the caller must `release` after the per-session
    /// loop, or `None` if any CString conversion fails.
    ///
    /// typeName is the lowercase `"string"` literal: the shim matches it with
    /// `case "string":` (FoundationModelsCBindings.swift) to produce a
    /// `String`-typed property. Any other casing (e.g. "String") falls through
    /// to the "reference to another schema" branch and fails to build. The
    /// `anyOf` guide is added UNWRAPPED (`wrapped=false`): for a scalar String
    /// the shim's `resolveStringGuides` handles `.anyOf` directly, whereas a
    /// wrapped (`.element`) guide is only valid for array types and would throw
    /// `unsupportedGuide`.
    fn build_schema(api: &Api) -> Option<FMRef> {
        // typeName literal the shim maps to a Swift `String` property.
        let type_string = CString::new("string").ok()?;
        let schema_name = CString::new("SessionSummary").ok()?;
        let schema = unsafe { (api.schema_create)(schema_name.as_ptr(), std::ptr::null()) };
        if schema.is_null() {
            return None;
        }

        // Helper: create a property, optionally constrain it to an enum set via
        // an unwrapped anyOf guide, add it to the schema, then release the
        // property's +1 retain (the builder copied it into its own array).
        let add_prop = |name: &str, choices: Option<&[&str]>| -> Option<()> {
            let name_c = CString::new(name).ok()?;
            let prop = unsafe {
                (api.property_create)(
                    name_c.as_ptr(),
                    std::ptr::null(),
                    type_string.as_ptr(),
                    false,
                )
            };
            if prop.is_null() {
                return None;
            }
            if let Some(choices) = choices {
                // Keep the CStrings alive until after the FFI call.
                let owned: Vec<CString> = choices
                    .iter()
                    .map(|c| CString::new(*c))
                    .collect::<Result<_, _>>()
                    .ok()?;
                let ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
                unsafe {
                    (api.property_add_anyof)(prop, ptrs.as_ptr(), ptrs.len() as c_int, false);
                }
            }
            unsafe {
                (api.schema_add_property)(schema, prop);
                // Builder holds its own strong ref; release our creation +1.
                (api.release)(prop);
            }
            Some(())
        };

        // On any property failure, release the schema and bail.
        let built = (|| {
            add_prop("title", None)?;
            add_prop("description", None)?;
            add_prop("task_category", Some(VALID_CATEGORIES))?;
            add_prop("complexity", Some(VALID_COMPLEXITIES))?;
            Some(())
        })();
        if built.is_none() {
            unsafe { (api.release)(schema) };
            return None;
        }

        Some(schema)
    }

    /// Run a single structured generation for `input`, blocking the calling
    /// thread until the background callback fires. Returns the parsed summary,
    /// or `None` on any error (caller falls back to the heuristic).
    ///
    /// `schema` is the prebuilt, shared GenerationSchema ref (see
    /// [`build_schema`]); the shim borrows it unretained per call, so it stays
    /// owned by the caller across the loop.
    ///
    /// A FRESH `LanguageModelSession` is created per input: the session is
    /// stateful (it accumulates a transcript), so reusing one across sessions
    /// would condition later summaries on earlier prompts/responses — and a
    /// timed-out generation could leave a shared session busy. This mirrors the
    /// former Python backend, which built a new session inside its loop.
    fn respond_one(
        api: &Api,
        model: FMRef,
        instructions: &CStr,
        schema: FMRef,
        input: &SessionInput,
    ) -> Option<SessionSummary> {
        let session_ref =
            unsafe { (api.session_create)(model, instructions.as_ptr(), std::ptr::null_mut(), 0) };
        if session_ref.is_null() {
            return None;
        }

        // Build the prompt CString BEFORE allocating the composed-prompt handle,
        // so an unexpected NUL byte cannot leak an allocated FM handle.
        let prompt_text = match CString::new(build_prompt(input)) {
            Ok(c) => c,
            Err(_) => {
                unsafe { (api.release)(session_ref) };
                return None;
            }
        };
        let prompt_ref = unsafe { (api.prompt_init)() };
        if prompt_ref.is_null() {
            unsafe { (api.release)(session_ref) };
            return None;
        }
        unsafe { (api.prompt_add_text)(prompt_ref, prompt_text.as_ptr()) };

        let (tx, rx) = mpsc::channel::<CallbackResult>();
        let cb_box = Box::new(CallbackBox { tx });
        let user_info = Box::into_raw(cb_box) as *mut c_void;

        let task_ref = unsafe {
            (api.respond_with_schema)(
                session_ref,
                prompt_ref,
                schema,
                std::ptr::null(),
                user_info,
                structured_callback,
            )
        };

        // Block on the background callback (bounded). The callback reclaims
        // `user_info` (the boxed sender). On timeout we deliberately do NOT
        // reclaim it here: the detached Swift task may still fire the callback
        // later, so freeing the box now would risk a use-after-free. The box
        // (one channel sender) is leaked instead — a bounded, rare cost paid
        // only when a generation exceeds the 60s timeout.
        let received = rx.recv_timeout(FM_GENERATION_TIMEOUT);

        // Release the task handle, composed prompt, and this input's session.
        if !task_ref.is_null() {
            unsafe { (api.release)(task_ref) };
        }
        unsafe { (api.release)(prompt_ref) };
        unsafe { (api.release)(session_ref) };

        match received {
            Ok(Ok(json)) => parse_summary(&input.session_id, &json),
            // Surface the failure mode so silent degradation to the heuristic is
            // diagnosable (the FM-vs-heuristic breakdown reports the count; this
            // names the cause). Non-success status codes flow through here.
            Ok(Err(status)) => {
                eprintln!(
                    "  apple-fm: generation failed for {} (status {}); using heuristic",
                    input.session_id, status
                );
                None
            }
            Err(_) => {
                eprintln!(
                    "  apple-fm: generation timed out for {} after {}s; using heuristic",
                    input.session_id,
                    FM_GENERATION_TIMEOUT.as_secs()
                );
                None
            }
        }
    }

    /// Real FFI implementation. See module docs and [`super::summarize`].
    pub fn summarize(sessions: &[SessionInput]) -> Option<Vec<SessionSummary>> {
        if sessions.is_empty() {
            return Some(Vec::new());
        }

        // 0) Resolve the dylib + entry points (OS gate + dlopen happen here).
        //    `None` => old macOS / dylib missing => caller uses the heuristic.
        let api = api()?;

        // 1) Default model + availability gate. NEVER generate if unavailable.
        let model = unsafe { (api.get_default)() };
        if model.is_null() {
            return None;
        }
        let available = unsafe { (api.is_available)(model, std::ptr::null_mut()) };
        if !available {
            unsafe { (api.release)(model) };
            return None;
        }

        // 2) Prepare the shared instructions + output schema once. respond_one
        //    creates a FRESH session per input from these (see its docs).
        let instructions = match CString::new(SYSTEM_INSTRUCTIONS) {
            Ok(c) => c,
            Err(_) => {
                unsafe { (api.release)(model) };
                return None;
            }
        };
        // Build the programmatic output schema ONCE and share it across the
        // per-session loop (the shim borrows it unretained per call). Released
        // after the loop. If schema construction fails, fall back entirely.
        let schema = match build_schema(api) {
            Some(s) => s,
            None => {
                unsafe { (api.release)(model) };
                return None;
            }
        };

        // 3) One structured generation per session; per-session errors fall
        //    back to the heuristic for that single session.
        let mut results = Vec::with_capacity(sessions.len());
        for input in sessions {
            match respond_one(api, model, instructions.as_c_str(), schema, input) {
                Some(summary) => results.push(summary),
                None => results.push(heuristic_classify(input)),
            }
        }

        unsafe {
            (api.release)(schema);
            (api.release)(model);
        }

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

    /// Live end-to-end check against the real on-device model. Kept `#[ignore]`d
    /// so it never runs in CI (it requires Apple Intelligence enabled + the
    /// on-device model READY). Run manually with:
    ///   cargo test -p tokscale-cli --features apple-fm -- --ignored live_summarize_smoke
    /// Documents the live path; use THIS, not the segfaulting fm-c-example
    /// streaming binary, as the on-device smoke test on macOS 26.x.
    #[cfg(all(target_os = "macos", feature = "apple-fm"))]
    #[test]
    #[ignore]
    fn live_summarize_smoke() {
        let sessions = vec![
            SessionInput {
                session_id: "ses_live_1".to_string(),
                client: "claude".to_string(),
                workspace: "/Users/x/payments-api".to_string(),
                first_user_message: Some(
                    "Add JWT auth middleware to the payments API and write tests.".to_string(),
                ),
                models_used: vec!["claude-opus-4".to_string()],
                total_tokens: 120_000,
                duration_minutes: 45,
                message_count: 12,
            },
            SessionInput {
                session_id: "ses_live_2".to_string(),
                client: "claude".to_string(),
                workspace: "/Users/x/dashboard".to_string(),
                first_user_message: Some(
                    "The settings page crashes with a null pointer when the avatar URL is empty; \
                     find and fix the bug."
                        .to_string(),
                ),
                models_used: vec!["claude-haiku-4".to_string()],
                total_tokens: 8_000,
                duration_minutes: 10,
                message_count: 4,
            },
        ];

        let out = summarize(&sessions).expect("FM should be available on this box");
        assert_eq!(out.len(), 2);
        for s in &out {
            eprintln!(
                "live[{}]: title={:?} category={:?} complexity={:?} fm_version={:?}\n  desc={:?}",
                s.session_id, s.title, s.task_category, s.complexity, s.fm_version, s.description
            );
        }
        // After a working schema/generation, every summary must be FM-produced,
        // not the heuristic backfill.
        for s in &out {
            assert_eq!(
                s.fm_version.as_deref(),
                Some("apple-fm-on-device"),
                "expected FM-generated provenance, got heuristic fallback for {}",
                s.session_id
            );
            assert!(VALID_CATEGORIES.contains(&s.task_category.as_str()));
            assert!(VALID_COMPLEXITIES.contains(&s.complexity.as_str()));
        }
    }
}
