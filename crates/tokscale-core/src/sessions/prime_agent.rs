//! Prime Agent session parser.
//!
//! Prime Agent stores root sessions in `~/.prime/agent/sessions/*.jsonl` and
//! RLM child sessions below the sibling `session-artifacts` tree. Both use the
//! Pi append-only JSONL record format, so token extraction is shared with the
//! Pi parser. `child_usage_attributed` records are never emitted as messages:
//! tokscale scans each child's own transcript directly. Their usage metadata is
//! used only to reverse aggregate parent usage that Prime may persist while
//! serializing a fork, before the copied parent is deduplicated across files.

use super::pi::{
    has_replacement_character, parse_pi_format_rlm_file_with_observer,
    pre_header_line_is_skippable, raw_json_has_damaged_lineage_header_key, rlm_entry_emits_message,
    PiFormatObserver, PiSessionEntry, PiSessionHeader, PiUsage, PRE_SESSION_METADATA_TYPES,
};
use super::utils::{lossy_lines_with_bytes, parse_timestamp_str};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::BufReader;
use std::path::{Path, PathBuf};

#[cfg(test)]
#[derive(Default)]
struct PrimeDecodeCounter {
    root: Option<PathBuf>,
    messages: usize,
    accounting: usize,
}

#[cfg(test)]
static PRIME_DECODE_COUNTER: std::sync::LazyLock<std::sync::Mutex<PrimeDecodeCounter>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(PrimeDecodeCounter::default()));

#[cfg(test)]
static ACCOUNTING_BACKFILL_REWRITE: std::sync::LazyLock<
    std::sync::Mutex<Option<(PathBuf, String)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

#[cfg(test)]
static STABLE_PARSE_REWRITE: std::sync::LazyLock<std::sync::Mutex<Option<(PathBuf, String)>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

#[cfg(test)]
pub(crate) fn schedule_accounting_backfill_test_rewrite(path: &Path, contents: String) {
    *ACCOUNTING_BACKFILL_REWRITE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((path.to_path_buf(), contents));
}

#[cfg(test)]
pub(crate) fn schedule_stable_parse_test_rewrite(path: &Path, contents: String) {
    *STABLE_PARSE_REWRITE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((path.to_path_buf(), contents));
}

#[cfg(test)]
pub(crate) fn run_accounting_backfill_test_hook(path: &Path) {
    let mut scheduled = ACCOUNTING_BACKFILL_REWRITE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if scheduled
        .as_ref()
        .is_some_and(|(scheduled_path, _)| scheduled_path == path)
    {
        let (_, contents) = scheduled.take().unwrap();
        let modified = std::fs::metadata(path).unwrap().modified().unwrap();
        std::fs::write(path, contents).unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }
}

#[cfg(test)]
pub(crate) fn run_stable_parse_test_hook(path: &Path) {
    let mut scheduled = STABLE_PARSE_REWRITE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if scheduled
        .as_ref()
        .is_some_and(|(scheduled_path, _)| scheduled_path == path)
    {
        let (_, contents) = scheduled.take().unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

#[cfg(test)]
fn record_transcript_decode(path: &Path, accounting: bool) {
    let mut counter = PRIME_DECODE_COUNTER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if counter
        .root
        .as_deref()
        .is_some_and(|root| path.starts_with(root))
    {
        if accounting {
            counter.accounting += 1;
        } else {
            counter.messages += 1;
        }
    }
}

pub fn parse_prime_agent_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_prime_agent_file_with_accounting(path).0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PrimeAttribution {
    id: String,
    timestamp: Option<i64>,
    child_usage: TokenBreakdown,
    aggregate_usage: TokenBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChildMessageUsage {
    timestamp: Option<i64>,
    usage: TokenBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PrimeUsageAdjustment {
    dedup_key: String,
    persisted_usage: TokenBreakdown,
    attributions: Vec<PrimeAttribution>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PrimeFileAccounting {
    source_path: PathBuf,
    attributions: Vec<PrimeAttribution>,
    adjustments: Vec<PrimeUsageAdjustment>,
    child_message_usages: Vec<ChildMessageUsage>,
    child_parent_path: Option<PathBuf>,
    fork_parent_path: Option<PathBuf>,
}

struct PrimeAccountingBuilder<'a> {
    path: &'a Path,
    found_header: bool,
    is_rlm_child: bool,
    child_parent_path: Option<PathBuf>,
    fork_parent_path: Option<PathBuf>,
    targets: HashMap<String, (String, TokenBreakdown)>,
    attributions: HashMap<String, Vec<PrimeAttribution>>,
    child_message_usages: Vec<ChildMessageUsage>,
}

impl<'a> PrimeAccountingBuilder<'a> {
    fn new(path: &'a Path) -> Self {
        Self {
            path,
            found_header: false,
            is_rlm_child: false,
            child_parent_path: None,
            fork_parent_path: None,
            targets: HashMap::new(),
            attributions: HashMap::new(),
            child_message_usages: Vec::new(),
        }
    }

    fn finish(self) -> PrimeFileAccounting {
        if !self.found_header {
            return PrimeFileAccounting::default();
        }

        let all_attributions = self
            .attributions
            .values()
            .flat_map(|entries| entries.iter().cloned())
            .collect();
        let mut adjustments = Vec::new();
        for (target_id, entries) in self.attributions {
            let Some((dedup_key, persisted_usage)) = self.targets.get(&target_id) else {
                continue;
            };
            let mut matching_prefix = None;
            for (index, entry) in entries.iter().enumerate() {
                if entry.aggregate_usage == *persisted_usage {
                    matching_prefix = Some(entries[..=index].to_vec());
                }
            }
            if let Some(prefix) = matching_prefix {
                adjustments.push(PrimeUsageAdjustment {
                    dedup_key: dedup_key.clone(),
                    persisted_usage: persisted_usage.clone(),
                    attributions: prefix,
                });
            }
        }

        PrimeFileAccounting {
            source_path: lineage_path(self.path),
            attributions: all_attributions,
            adjustments,
            child_message_usages: self.child_message_usages,
            child_parent_path: self.child_parent_path,
            fork_parent_path: self.fork_parent_path,
        }
    }
}

impl PiFormatObserver for PrimeAccountingBuilder<'_> {
    fn observe_header(&mut self, header: &PiSessionHeader) {
        self.found_header = true;
        self.is_rlm_child = header.rlm_depth.unwrap_or(0) > 0;
        let parent_path = header
            .parent_session
            .as_deref()
            .filter(|parent| !has_replacement_character(parent))
            .map(Path::new)
            .map(|parent| referenced_lineage_path(self.path, parent));
        if self.is_rlm_child {
            self.child_parent_path = parent_path;
        } else {
            self.fork_parent_path = parent_path;
        }
    }

    fn observe_entry(&mut self, entry: &PiSessionEntry, emitted: Option<&UnifiedMessage>) {
        let entry_timestamp = entry.timestamp.as_deref().and_then(parse_timestamp_str);
        if entry.entry_type == "child_usage_attributed" {
            if let (Some(id), Some(target_id), Some(child_usage), Some(aggregate_usage)) = (
                entry.id.as_ref(),
                entry.target_id.as_ref(),
                entry.child_usage.as_ref(),
                entry.aggregate_usage.as_ref(),
            ) {
                if has_replacement_character(id)
                    || has_replacement_character(target_id)
                    || child_usage.has_damaged_key()
                    || aggregate_usage.has_damaged_key()
                {
                    return;
                }
                self.attributions
                    .entry(target_id.clone())
                    .or_default()
                    .push(PrimeAttribution {
                        id: id.clone(),
                        timestamp: entry_timestamp,
                        child_usage: usage_breakdown(child_usage),
                        aggregate_usage: usage_breakdown(aggregate_usage),
                    });
            }
            return;
        }

        let Some(parsed) = emitted else {
            return;
        };
        if self.is_rlm_child {
            self.child_message_usages.push(ChildMessageUsage {
                timestamp: entry_timestamp,
                usage: parsed.tokens.clone(),
            });
        }
        if let (Some(id), Some(dedup_key)) = (entry.id.as_ref(), parsed.dedup_key.as_ref()) {
            if !has_replacement_character(id) {
                self.targets
                    .insert(id.clone(), (dedup_key.clone(), parsed.tokens.clone()));
            }
        }
    }
}

pub(crate) fn parse_prime_agent_file_with_accounting(
    path: &Path,
) -> (Vec<UnifiedMessage>, PrimeFileAccounting) {
    #[cfg(test)]
    record_transcript_decode(path, false);

    let mut accounting = PrimeAccountingBuilder::new(path);
    let messages =
        parse_pi_format_rlm_file_with_observer(path, "prime-agent", "prime-agent", &mut accounting);
    (messages, accounting.finish())
}

#[cfg(test)]
pub(crate) fn reset_transcript_decode_call_counts(root: &Path) {
    *PRIME_DECODE_COUNTER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = PrimeDecodeCounter {
        root: Some(root.to_path_buf()),
        messages: 0,
        accounting: 0,
    };
}

#[cfg(test)]
pub(crate) fn transcript_decode_call_counts() -> (usize, usize) {
    let counter = PRIME_DECODE_COUNTER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    (counter.messages, counter.accounting)
}

fn usage_breakdown(usage: &PiUsage) -> TokenBreakdown {
    TokenBreakdown {
        input: usage.input.unwrap_or(0).max(0),
        output: usage.output.unwrap_or(0).max(0),
        cache_read: usage.cache_read.unwrap_or(0).max(0),
        cache_write: usage.cache_write.unwrap_or(0).max(0),
        reasoning: 0,
    }
}

fn add_usage(total: &mut TokenBreakdown, usage: &TokenBreakdown) {
    *total += usage;
}

// Prime accounting snapshots currently expose only these four cumulative
// fields. This is an intentional field-wise max, not additive aggregation;
// reasoning remains zero until the transcript schema provides it.
fn maximize_usage(total: &mut TokenBreakdown, usage: &TokenBreakdown) {
    total.input = total.input.max(usage.input);
    total.output = total.output.max(usage.output);
    total.cache_read = total.cache_read.max(usage.cache_read);
    total.cache_write = total.cache_write.max(usage.cache_write);
}

// Residual accounting subtracts the same four cumulative snapshot fields.
// This is deliberately not whole-breakdown addition.
fn subtract_usage(total: &mut TokenBreakdown, usage: &TokenBreakdown) {
    total.input = total.input.saturating_sub(usage.input).max(0);
    total.output = total.output.saturating_sub(usage.output).max(0);
    total.cache_read = total.cache_read.saturating_sub(usage.cache_read).max(0);
    total.cache_write = total.cache_write.saturating_sub(usage.cache_write).max(0);
}

type UsageKey = (i64, i64, i64, i64);
type LineageUsageKey = (PathBuf, UsageKey);
/// Attribution ids are only unique within one session: Prime mints them with
/// `randomUUID().slice(0, 8)` and collision-checks against that session's own id
/// map alone. Pairing an id with its resolved lineage root keeps fork copies of
/// one attribution collapsed while keeping a colliding id in an unrelated
/// lineage independent.
type AttributionKey = (PathBuf, String);
/// One parsed child response: the pool bucket it landed in plus its position
/// inside that bucket. Buckets are keyed by parent lineage and usage, so this
/// identifies a single transcript entry without depending on scan order.
type ChildResponseRef = (LineageUsageKey, usize);

fn usage_key(usage: &TokenBreakdown) -> UsageKey {
    (
        usage.input,
        usage.output,
        usage.cache_read,
        usage.cache_write,
    )
}

/// Resolve every file to the head of its fork chain. Serializing a fork copies
/// the parent's `child_usage_attributed` records verbatim, so all copies within
/// one chain describe the same invocation and must share an attribution
/// identity. Files in different chains never do.
///
/// A chain can loop: two files can name each other as fork parent, and a rewritten
/// or relocated session can close a longer loop. Stopping the walk on a repeat is
/// not enough, because each member would then stop at itself and the copies would
/// be accounted for as unrelated attributions, restoring the same child delta once
/// per member. Every member of a loop therefore resolves to a single deterministic
/// representative instead.
fn lineage_roots(accounting: &[PrimeFileAccounting]) -> HashMap<PathBuf, PathBuf> {
    let forked_from: HashMap<&PathBuf, &PathBuf> = accounting
        .iter()
        .filter_map(|file| Some((&file.source_path, file.fork_parent_path.as_ref()?)))
        .collect();
    let mut roots: HashMap<PathBuf, PathBuf> = HashMap::new();
    for file in accounting {
        // Walk the fork chain, remembering the order the files were seen so a
        // cycle can be recognized by where it closes rather than merely stopped.
        let mut chain: Vec<PathBuf> = Vec::new();
        let mut position: HashMap<PathBuf, usize> = HashMap::new();
        let mut node = file.source_path.clone();
        let root = loop {
            if let Some(resolved) = roots.get(&node) {
                break resolved.clone();
            }
            if let Some(entered) = position.get(&node).copied() {
                // A fork chain that loops back on itself: every file in the loop
                // is a copy of the same fork history, so they must collapse onto
                // one representative instead of each becoming its own root. Take
                // the smallest path in the loop, which no scan order can change.
                break chain[entered..].iter().min().cloned().unwrap_or(node);
            }
            position.insert(node.clone(), chain.len());
            chain.push(node.clone());
            match forked_from.get(&node) {
                Some(parent) => node = (*parent).clone(),
                // The head of an acyclic chain is its own root.
                None => break node,
            }
        };
        // Memoized for the whole walk: every file on a chain shares its root, and
        // a chain that runs into a loop adopts the loop's representative.
        for member in chain {
            roots.insert(member, root.clone());
        }
        roots.entry(file.source_path.clone()).or_insert(root);
    }
    roots
}

fn lineage_root(roots: &HashMap<PathBuf, PathBuf>, file: &PrimeFileAccounting) -> PathBuf {
    roots
        .get(&file.source_path)
        .cloned()
        .unwrap_or_else(|| file.source_path.clone())
}

fn lineage_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn referenced_lineage_path(source_file: &Path, referenced: &Path) -> PathBuf {
    if referenced.is_absolute() {
        lineage_path(referenced)
    } else {
        lineage_path(
            &source_file
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(referenced),
        )
    }
}

fn parse_pi_json_line<T: DeserializeOwned>(line: &str, buffer: &mut Vec<u8>) -> Option<T> {
    buffer.clear();
    buffer.extend_from_slice(line.as_bytes());
    simd_json::from_slice(buffer).ok()
}

/// Read Prime-only accounting records that are intentionally absent from the
/// shared Pi message representation. `messages` may come from the source cache;
/// their stable order is used to associate target entry ids with emitted rows.
pub(crate) fn analyze_prime_agent_accounting(
    path: &Path,
    messages: &[UnifiedMessage],
) -> PrimeFileAccounting {
    #[cfg(test)]
    record_transcript_decode(path, true);

    let Ok(file) = std::fs::File::open(path) else {
        return PrimeFileAccounting::default();
    };

    let mut accounting = PrimeAccountingBuilder::new(path);
    let mut found_header = false;
    let mut message_index = 0usize;
    let mut buffer = Vec::with_capacity(4096);
    for line in lossy_lines_with_bytes(BufReader::new(file)) {
        let trimmed = line.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !found_header {
            if let Some(header) = parse_pi_json_line::<PiSessionHeader>(trimmed, &mut buffer) {
                if header.entry_type == "session" {
                    let has_raw_damaged_lineage_key =
                        raw_json_has_damaged_lineage_header_key(&line.bytes);
                    if header.has_invalid_lineage() || has_raw_damaged_lineage_key {
                        return PrimeFileAccounting::default();
                    }
                    found_header = true;
                    accounting.observe_header(&header);
                    continue;
                }
            }
            let parsed_type = parse_pi_json_line::<serde_json::Value>(trimmed, &mut buffer)
                .and_then(|value| {
                    value
                        .get("type")
                        .and_then(|kind| kind.as_str())
                        .map(str::to_owned)
                });
            let is_pre_session_metadata = parsed_type
                .as_deref()
                .is_some_and(|kind| PRE_SESSION_METADATA_TYPES.contains(&kind));
            if is_pre_session_metadata
                || pre_header_line_is_skippable(trimmed, parsed_type.as_deref())
            {
                continue;
            }
            return PrimeFileAccounting::default();
        }

        let Some(entry) = parse_pi_json_line::<PiSessionEntry>(trimmed, &mut buffer) else {
            continue;
        };
        // Which records became messages is the parser's decision, not a rule
        // restated here: a record counted on only one side shifts every later
        // index of `messages` and re-targets the reconciliation below.
        let emitted = rlm_entry_emits_message(&entry, accounting.is_rlm_child)
            .then(|| {
                let parsed = messages.get(message_index);
                message_index += 1;
                parsed
            })
            .flatten();
        accounting.observe_entry(&entry, emitted);
    }

    accounting.finish()
}

fn fallback_key_base(key: &str) -> Option<&str> {
    if !key.starts_with("prime-agent:message:") {
        return None;
    }
    let mut parts = key.rsplitn(5, ':');
    parts.next()?;
    parts.next()?;
    parts.next()?;
    parts.next()?;
    parts.next()
}

fn rewrite_fallback_usage(key: &str, usage: &TokenBreakdown) -> String {
    fallback_key_base(key).map_or_else(
        || key.to_string(),
        |base| {
            format!(
                "{base}:{}:{}:{}:{}",
                usage.input, usage.output, usage.cache_read, usage.cache_write
            )
        },
    )
}

/// Timestamp distance in milliseconds between an attribution and a parsed child
/// response. A lower cost is a better explanation of one completion event.
type MatchCost = i64;

/// One independent contention group, in dense local indices: the attributions
/// that reach a shared set of child responses, directly or transitively.
/// Separate components never influence each other, so each is matched alone.
struct MatchingComponent {
    /// Local attribution index -> index into the global attribution key list.
    attributions: Vec<usize>,
    /// Local attribution index -> its eligible (local child index, cost) pairs.
    edges: Vec<Vec<(usize, MatchCost)>>,
    children: usize,
}

fn disjoint_set_root(parents: &mut [usize], node: usize) -> usize {
    let mut root = node;
    while parents[root] != root {
        root = parents[root];
    }
    let mut walk = node;
    while parents[walk] != root {
        let next = parents[walk];
        parents[walk] = root;
        walk = next;
    }
    root
}

/// Split the attribution/child graph into connected components, so the
/// attributions that genuinely contend for the same child responses are matched
/// together while unrelated sessions stay out of each other's cost accounting.
fn matching_components(eligible: &[Vec<(MatchCost, ChildResponseRef)>]) -> Vec<MatchingComponent> {
    let mut child_indices: BTreeMap<ChildResponseRef, usize> = BTreeMap::new();
    for candidates in eligible {
        for (_, candidate) in candidates {
            let next = child_indices.len();
            child_indices.entry(candidate.clone()).or_insert(next);
        }
    }
    let mut parents: Vec<usize> = (0..eligible.len() + child_indices.len()).collect();
    for (attribution, candidates) in eligible.iter().enumerate() {
        for (_, candidate) in candidates {
            let child = eligible.len() + child_indices[candidate];
            let left = disjoint_set_root(&mut parents, attribution);
            let right = disjoint_set_root(&mut parents, child);
            if left != right {
                parents[left] = right;
            }
        }
    }
    let mut grouped: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (attribution, candidates) in eligible.iter().enumerate() {
        if candidates.is_empty() {
            continue;
        }
        let root = disjoint_set_root(&mut parents, attribution);
        grouped.entry(root).or_default().push(attribution);
    }
    grouped
        .into_values()
        .map(|attributions| {
            let mut local_children: BTreeMap<ChildResponseRef, usize> = BTreeMap::new();
            let mut edges = Vec::with_capacity(attributions.len());
            for attribution in &attributions {
                let mut candidates = Vec::new();
                for (cost, candidate) in &eligible[*attribution] {
                    let next = local_children.len();
                    let child = *local_children.entry(candidate.clone()).or_insert(next);
                    candidates.push((child, *cost));
                }
                edges.push(candidates);
            }
            MatchingComponent {
                attributions,
                edges,
                children: local_children.len(),
            }
        })
        .collect()
}

/// Minimum-cost maximum matching over one component, by successive shortest
/// augmenting paths: every augmentation takes the cheapest path that adds one
/// pair, so the result is a maximum matching whose total timestamp distance is
/// the smallest of any maximum matching. Plain maximum-cardinality matching is
/// not enough here -- it fixes how many attributions are matched but not which
/// ones -- so this is what stops an attribution that merely lands inside the
/// tolerance window from consuming a child response another attribution explains
/// exactly.
///
/// `blocked` removes one attribution from the component, which answers whether
/// that attribution is dispensable at no extra cost by brute force. Production
/// code derives that from a single matching via `indispensable_attributions`;
/// the parameter is kept so the tests can check the fast derivation against the
/// definition it implements.
///
/// Returns the cardinality, the total cost, and each local attribution's child.
fn min_cost_max_matching(
    component: &MatchingComponent,
    blocked: Option<usize>,
) -> (usize, MatchCost, Vec<Option<usize>>) {
    let attributions = component.edges.len();
    let children = component.children;
    let source = attributions + children;
    let sink = source + 1;
    let mut matched_attribution: Vec<Option<usize>> = vec![None; attributions];
    let mut matched_child: Vec<Option<usize>> = vec![None; children];
    let mut cardinality = 0usize;

    loop {
        // Residual arcs: an unused pairing costs its distance and a used one
        // refunds it, so the cheapest source-to-sink walk is the cheapest way
        // to gain one pair. Refunds are negative, hence Bellman-Ford rather
        // than Dijkstra; a component is the handful of attributions sharing one
        // equal-usage bucket inside one lineage.
        let mut residual: Vec<(usize, usize, MatchCost)> = Vec::new();
        for (attribution, matched) in matched_attribution.iter().enumerate() {
            if blocked == Some(attribution) {
                continue;
            }
            if matched.is_none() {
                residual.push((source, attribution, 0));
            }
            for &(child, cost) in &component.edges[attribution] {
                if *matched == Some(child) {
                    residual.push((attributions + child, attribution, -cost));
                } else {
                    residual.push((attribution, attributions + child, cost));
                }
            }
        }
        for (child, matched) in matched_child.iter().enumerate() {
            if matched.is_none() {
                residual.push((attributions + child, sink, 0));
            }
        }

        let mut distance: Vec<Option<MatchCost>> = vec![None; sink + 1];
        let mut previous: Vec<Option<usize>> = vec![None; sink + 1];
        distance[source] = Some(0);
        for _ in 0..=sink {
            let mut improved = false;
            for &(from, to, cost) in &residual {
                let Some(reached) = distance[from] else {
                    continue;
                };
                let candidate = reached + cost;
                if distance[to].is_none_or(|current| candidate < current) {
                    distance[to] = Some(candidate);
                    previous[to] = Some(from);
                    improved = true;
                }
            }
            if !improved {
                break;
            }
        }
        if distance[sink].is_none() {
            break;
        }

        // Re-seat every pairing the augmenting path crosses. A shortest path is
        // simple, so each attribution and each child response appears at most
        // once and the rewrites are independent of the order applied.
        let mut node = sink;
        let mut steps = 0;
        while let Some(from) = previous[node] {
            if from < attributions && (attributions..source).contains(&node) {
                let child = node - attributions;
                matched_attribution[from] = Some(child);
                matched_child[child] = Some(from);
            }
            node = from;
            steps += 1;
            if steps > sink {
                break;
            }
        }
        cardinality += 1;
    }

    let mut cost = 0;
    for (attribution, matched) in matched_attribution.iter().enumerate() {
        if let Some(child) = matched {
            cost += component.edges[attribution]
                .iter()
                .find(|(candidate, _)| candidate == child)
                .map_or(0, |(_, cost)| *cost);
        }
    }
    (cardinality, cost, matched_attribution)
}

/// Which attributions appear in EVERY minimum-cost maximum matching of the
/// component, derived from a single matching instead of re-solving the whole
/// component once per matched attribution.
///
/// Read the matching as a unit-capacity min-cost flow of value `cardinality`:
/// `source -> attribution -> child -> sink`. Any other matching of the same
/// cardinality and the same cost is that flow plus a zero-cost circulation in
/// the residual graph, and every such circulation splits into simple residual
/// cycles that individually cost zero. Node potentials that make all residual
/// arcs non-negative exist because the matching is already cost-optimal, and
/// potentials cancel around a cycle, so a residual cycle costs zero exactly
/// when every arc on it has zero reduced cost.
///
/// A matched attribution drops out of the matching precisely when such a cycle
/// cancels its `source -> attribution` arc, i.e. when the residual arc
/// `attribution -> source` has zero reduced cost and lies on a zero-reduced-cost
/// cycle. Because that arc leads to `source`, the cycle exists exactly when
/// `source` reaches the attribution over zero-reduced-cost residual arcs. So one
/// Bellman-Ford pass for the potentials plus one traversal answers the question
/// for every attribution at once.
fn indispensable_attributions(
    component: &MatchingComponent,
    matched_attribution: &[Option<usize>],
) -> Vec<bool> {
    let attributions = component.edges.len();
    let children = component.children;
    let source = attributions + children;
    let sink = source + 1;
    let nodes = sink + 1;

    let mut matched_child: Vec<Option<usize>> = vec![None; children];
    for (attribution, matched) in matched_attribution.iter().enumerate() {
        if let Some(child) = matched {
            matched_child[*child] = Some(attribution);
        }
    }

    // The complete residual graph, unlike the augmenting-path search, which can
    // skip the arcs back to the source and out of the sink because a shortest
    // path never takes them. A cycle that re-seats which child feeds the sink
    // does take them, and that cycle can free an attribution, so they matter
    // here.
    let mut residual: Vec<(usize, usize, MatchCost)> = Vec::new();
    for (attribution, matched) in matched_attribution.iter().enumerate() {
        match matched {
            None => residual.push((source, attribution, 0)),
            Some(_) => residual.push((attribution, source, 0)),
        }
        for &(child, cost) in &component.edges[attribution] {
            if *matched == Some(child) {
                residual.push((attributions + child, attribution, -cost));
            } else {
                residual.push((attribution, attributions + child, cost));
            }
        }
    }
    for (child, matched) in matched_child.iter().enumerate() {
        match matched {
            None => residual.push((attributions + child, sink, 0)),
            Some(_) => residual.push((sink, attributions + child, 0)),
        }
    }

    // Potentials, as Bellman-Ford from a virtual node joined to every node at
    // zero cost: `potential[v] <= potential[u] + cost(u, v)` on every residual
    // arc is exactly the non-negative reduced cost the argument above needs.
    // The relaxation converges because a cost-optimal flow leaves no negative
    // residual cycle.
    let mut potential: Vec<MatchCost> = vec![0; nodes];
    for _ in 0..nodes {
        let mut improved = false;
        for &(from, to, cost) in &residual {
            let candidate = potential[from] + cost;
            if candidate < potential[to] {
                potential[to] = candidate;
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
    // `<= 0` rather than `== 0`: with converged potentials the two agree, and if
    // they ever disagreed the extra arcs would only widen the set of
    // attributions treated as dispensable, which is the conservative direction
    // -- an unmatched attribution keeps its parent aggregate rather than
    // authorizing a subtraction.
    let reduced_cost =
        |from: usize, to: usize, cost: MatchCost| cost + potential[from] - potential[to];

    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    for &(from, to, cost) in &residual {
        if reduced_cost(from, to, cost) <= 0 {
            adjacency[from].push(to);
        }
    }
    let mut reached = vec![false; nodes];
    reached[source] = true;
    let mut stack = vec![source];
    while let Some(node) = stack.pop() {
        for &next in &adjacency[node] {
            if !reached[next] {
                reached[next] = true;
                stack.push(next);
            }
        }
    }

    matched_attribution
        .iter()
        .enumerate()
        .map(|(attribution, matched)| {
            matched.is_some()
                && !(reached[attribution] && reduced_cost(attribution, source, 0) <= 0)
        })
        .collect()
}

/// Subtract child usage only when a matching RLM transcript was actually
/// parsed, then collapse fork copies. Missing/pruned children remain represented
/// by Prime's aggregate parent usage instead of disappearing from the total.
pub(crate) fn reconcile_prime_agent_messages(
    messages: Vec<UnifiedMessage>,
    accounting: &[PrimeFileAccounting],
) -> Vec<UnifiedMessage> {
    const ATTRIBUTION_TIMESTAMP_TOLERANCE_MS: i64 = 1_000;

    let mut available_children: HashMap<LineageUsageKey, Vec<Option<i64>>> = HashMap::new();
    for file in accounting {
        if let Some(parent_path) = &file.child_parent_path {
            for child in &file.child_message_usages {
                available_children
                    .entry((parent_path.clone(), usage_key(&child.usage)))
                    .or_default()
                    .push(child.timestamp);
            }
        }
    }

    // Attribution ids survive fork serialization. Record every file that owns
    // a copy, but match only a child response whose header points back to that
    // parent session and whose completion timestamp is the same event (Prime
    // writes the two records within milliseconds). This disambiguates equal
    // token buckets produced by separate children in one parent.
    // Attribution ids are unique only inside one session, so they are keyed by
    // their lineage root as well: fork copies of one attribution still collapse,
    // while a colliding id minted in an unrelated lineage stays separate.
    let roots = lineage_roots(accounting);
    let mut unique_attributions: BTreeMap<
        AttributionKey,
        (TokenBreakdown, Option<i64>, BTreeSet<PathBuf>),
    > = BTreeMap::new();
    for file in accounting {
        let lineage = lineage_root(&roots, file);
        for attribution in &file.attributions {
            let (_, _, owners) = unique_attributions
                .entry((lineage.clone(), attribution.id.clone()))
                .or_insert_with(|| {
                    (
                        attribution.child_usage.clone(),
                        attribution.timestamp,
                        BTreeSet::new(),
                    )
                });
            owners.insert(file.source_path.clone());
            owners.insert(lineage.clone());
            if let Some(parent) = &file.fork_parent_path {
                owners.insert(parent.clone());
            }
        }
    }

    // The matching rule, in full. An attribution authorizes subtracting its
    // `childUsage` from the parent aggregate only when a parsed child response
    // is matched to it, and a child response is eligible only when all three
    // hold:
    //
    // 1. Lineage and size. The child's `parentSession` header must resolve to a
    //    file that owns the attribution, and the child's usage must equal the
    //    recorded `childUsage` bucket exactly.
    // 2. Provable completion identity. Either both records carry a timestamp
    //    within ATTRIBUTION_TIMESTAMP_TOLERANCE_MS -- Prime appends the
    //    attribution milliseconds after the child response it describes -- or
    //    neither record carries one, which only happens in transcripts written
    //    before Prime timestamped its entries and where lineage plus size is
    //    the only identity that exists. A half-timed pairing proves nothing, so
    //    an unrelated same-sized sibling can never stand in for a pruned child
    //    and shrink the parent.
    // 3. Exclusivity. Matching is one-to-one: every child response authorizes
    //    at most one attribution and every attribution consumes at most one
    //    child response. N children of equal size completing in the same
    //    millisecond pair off with their N attributions rather than being
    //    discarded as ambiguous, which would count both the children and the
    //    parent aggregate that already contains them.
    //
    // Rule 3 is settled by a minimum-cost maximum matching, the cost of a
    // pairing being its timestamp distance. Maximum cardinality alone fixes how
    // MANY attributions are matched but not WHICH ones, and that choice decides
    // which parent response gets its aggregate reduced. Every attribution
    // contending for one child response carries the same `childUsage`, so the
    // global token total is the same for every maximum matching -- but the
    // per-model rows are not, and pricing is applied per model after
    // reconciliation, so an arbitrary choice silently moves cost between models.
    // Minimum cost keeps an attribution that merely lands inside the tolerance
    // window from consuming a child response another attribution explains
    // exactly.
    //
    // Remaining ties are resolved conservatively rather than arbitrarily. An
    // attribution is represented only when EVERY minimum-cost maximum matching
    // contains it; if an equally cheap matching exists that leaves it out, the
    // transcripts do not say which aggregate spent that child, so the aggregate
    // is retained -- the same fallback used for a child that was never parsed.
    // That rule is deterministic and independent of attribution id ordering. It
    // cannot decide the residual case where two attributions belonging to
    // different parent responses describe equally sized children that completed
    // in the very same millisecond: nothing in the records distinguishes them,
    // and proving identity there would need an upstream child or response id on
    // the attribution record.
    let attribution_keys: Vec<AttributionKey> = unique_attributions.keys().cloned().collect();
    let eligible: Vec<Vec<(MatchCost, ChildResponseRef)>> = unique_attributions
        .values()
        .map(|(usage, attribution_timestamp, owners)| {
            let mut candidates: Vec<(i64, ChildResponseRef)> = Vec::new();
            for owner in owners {
                let key = (owner.clone(), usage_key(usage));
                let Some(children) = available_children.get(&key) else {
                    continue;
                };
                for (index, child_timestamp) in children.iter().enumerate() {
                    match (attribution_timestamp, *child_timestamp) {
                        (Some(attribution), Some(child)) => {
                            let distance = attribution.abs_diff(child) as i64;
                            if distance <= ATTRIBUTION_TIMESTAMP_TOLERANCE_MS {
                                candidates.push((distance, (key.clone(), index)));
                            }
                        }
                        // Untimed on both sides: legacy transcripts, matched on
                        // lineage and size alone and ranked after every timed
                        // pairing.
                        (None, None) => candidates
                            .push((ATTRIBUTION_TIMESTAMP_TOLERANCE_MS + 1, (key.clone(), index))),
                        _ => {}
                    }
                }
            }
            candidates.sort();
            candidates
        })
        .collect();

    let mut represented_attributions: HashSet<AttributionKey> = HashSet::new();
    for component in matching_components(&eligible) {
        let (_, _, assignment) = min_cost_max_matching(&component, None);
        for (local, indispensable) in indispensable_attributions(&component, &assignment)
            .into_iter()
            .enumerate()
        {
            if indispensable {
                represented_attributions
                    .insert(attribution_keys[component.attributions[local]].clone());
            }
        }
    }

    let mut adjustment_groups: HashMap<String, Vec<(PathBuf, &PrimeUsageAdjustment)>> =
        HashMap::new();
    let mut attribution_fallback_bases = HashSet::new();
    for file in accounting {
        let lineage = lineage_root(&roots, file);
        for adjustment in &file.adjustments {
            let identity = fallback_key_base(&adjustment.dedup_key)
                .inspect(|base| {
                    attribution_fallback_bases.insert((*base).to_string());
                })
                .unwrap_or(&adjustment.dedup_key)
                .to_string();
            adjustment_groups
                .entry(identity)
                .or_default()
                .push((lineage.clone(), adjustment));
        }
    }

    let mut grouped: HashMap<String, Vec<UnifiedMessage>> = HashMap::new();
    let mut group_order = Vec::new();
    for (ordinal, message) in messages.into_iter().enumerate() {
        let identity = message.dedup_key.as_deref().map_or_else(
            || format!("prime-agent:unkeyed:{ordinal}"),
            |key| {
                fallback_key_base(key)
                    .filter(|base| attribution_fallback_bases.contains(*base))
                    .unwrap_or(key)
                    .to_string()
            },
        );
        if !grouped.contains_key(&identity) {
            group_order.push(identity.clone());
        }
        grouped.entry(identity).or_default().push(message);
    }

    let mut deduped = Vec::with_capacity(group_order.len());
    for identity in group_order {
        let mut group = grouped.remove(&identity).unwrap_or_default();
        let Some(mut representative) = group.first().cloned() else {
            continue;
        };
        let Some(adjustments) = adjustment_groups.get(&identity) else {
            for duplicate in group.iter().skip(1) {
                maximize_usage(&mut representative.tokens, &duplicate.tokens);
            }
            deduped.push(representative);
            continue;
        };

        let mut base_usage = TokenBreakdown::default();
        let mut found_base = false;
        let mut all_attributions: BTreeMap<AttributionKey, TokenBreakdown> = BTreeMap::new();
        for (lineage, adjustment) in adjustments {
            let mut own_usage = adjustment.persisted_usage.clone();
            for attribution in &adjustment.attributions {
                subtract_usage(&mut own_usage, &attribution.child_usage);
                all_attributions
                    .entry((lineage.clone(), attribution.id.clone()))
                    .or_insert_with(|| attribution.child_usage.clone());
            }
            maximize_usage(&mut base_usage, &own_usage);
            found_base = true;
        }
        for message in &group {
            let is_aggregate_copy = adjustments.iter().any(|(_, adjustment)| {
                message.dedup_key.as_deref() == Some(&adjustment.dedup_key)
                    && message.tokens == adjustment.persisted_usage
            });
            if !is_aggregate_copy {
                maximize_usage(&mut base_usage, &message.tokens);
                found_base = true;
            }
        }
        if !found_base {
            for message in &group {
                maximize_usage(&mut base_usage, &message.tokens);
            }
        }
        for (attribution_key, usage) in all_attributions {
            if !represented_attributions.contains(&attribution_key) {
                add_usage(&mut base_usage, &usage);
            }
        }

        representative.tokens = base_usage;
        if let Some(key) = representative.dedup_key.as_deref() {
            representative.dedup_key = Some(rewrite_fallback_usage(key, &representative.tokens));
        }
        group.clear();
        deduped.push(representative);
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn session_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn accepts_bom_crlf_and_later_records_after_invalid_utf8() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            b"\xef\xbb\xbf{\"type\":\"session\",\"version\":3,\"id\":\"root\",\"cwd\":\"/tmp/project\"}\r\n",
        )
        .unwrap();
        file.write_all(b"invalid \xff record\r\n").unwrap();
        file.write_all(
            br#"{"type":"message","id":"assistant","timestamp":"2026-08-08T00:00:01Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10}}}"#,
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_agent_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "root");
        assert_eq!(messages[0].tokens.total(), 180);
    }

    #[test]
    fn lossy_records_before_header_and_between_messages_preserve_accounting_alignment() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"garbage \xff before header\r\n").unwrap();
        file.write_all(
            br#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project"}
"#,
        )
        .unwrap();
        file.write_all(
            br#"{"type":"message","id":"assistant-1","timestamp":"2026-08-08T00:00:01Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-1","usage":{"input":10,"output":5}}}
"#,
        )
        .unwrap();
        file.write_all(b"damaged \xff record\r\n").unwrap();
        file.write_all(
            br#"{"type":"message","id":"assistant-2","timestamp":"2026-08-08T00:00:03Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-2","usage":{"input":20,"output":8}}}
"#,
        )
        .unwrap();
        file.write_all(
            br#"{"type":"child_usage_attributed","id":"usage-2","targetId":"assistant-2","childUsage":{"input":3,"output":2},"aggregateUsage":{"input":20,"output":8}}"#,
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_agent_file(file.path());
        let accounting = analyze_prime_agent_accounting(file.path(), &messages);

        assert_eq!(messages.len(), 2);
        assert_eq!(accounting.adjustments.len(), 1);
        assert_eq!(
            accounting.adjustments[0].dedup_key,
            "prime-agent:response:response-2"
        );
        assert_eq!(accounting.adjustments[0].attributions[0].id, "usage-2");
    }

    #[test]
    fn rejects_only_headers_with_damaged_lineage_keys() {
        for (prefix, suffix, rejected) in [
            // A replacement consumes at least one byte of a real lineage key.
            (b"parentSess".as_slice(), b"on".as_slice(), true),
            (b"rlmDep".as_slice(), b"h".as_slice(), true),
            (b"".as_slice(), b"arentSession".as_slice(), true),
            // These can only be damaged extension keys adjacent to, rather
            // than damaged spellings of, the complete structural key.
            // Raw-byte identity distinguishes invalid UTF-8 damage from a
            // clean literal replacement-bearing extension name.
            (b"parentSession".as_slice(), b"".as_slice(), true),
            (b"".as_slice(), b"parentSession".as_slice(), true),
            (b"parentSession".as_slice(), b"Extra".as_slice(), false),
            (b"xparentSess".as_slice(), b"on".as_slice(), false),
            (b"rlmDepth".as_slice(), b"".as_slice(), true),
            (br"rlmDepth".as_slice(), b"".as_slice(), true),
            (br"parentSession".as_slice(), b"".as_slice(), true),
            (b"extension".as_slice(), b"Field".as_slice(), false),
        ] {
            let mut file = NamedTempFile::new().unwrap();
            file.write_all(
                b"{\"type\":\"session\",\"version\":3,\"id\":\"child\",\"cwd\":\"/tmp/project\",\"",
            )
            .unwrap();
            file.write_all(prefix).unwrap();
            file.write_all(b"\xff").unwrap();
            file.write_all(suffix).unwrap();
            file.write_all(b"\":1}\n").unwrap();
            file.write_all(
                br#"{"type":"message","id":"assistant","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":10,"output":5}}}
"#,
            )
            .unwrap();
            file.flush().unwrap();

            let (messages, accounting) = parse_prime_agent_file_with_accounting(file.path());

            assert_eq!(messages.is_empty(), rejected);
            if rejected {
                assert!(accounting.attributions.is_empty());
                assert!(accounting.adjustments.is_empty());
                assert!(accounting.child_parent_path.is_none());
                assert!(accounting.fork_parent_path.is_none());
            }
        }

        let file = session_file(
            r#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project","extensionField":1}
{"type":"message","id":"assistant","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":10,"output":5}}}"#,
        );
        assert_eq!(parse_prime_agent_file(file.path()).len(), 1);

        for (encoded_key, rejected) in [
            (r"rlmDep\uFFFDth", true),
            (r#"unrelated\uD800":1,"rlmDep\uFFFDth"#, true),
            (r"parentSess\uFFFDon", true),
            (r"extension\uFFFDField", false),
            (r#"unrelated\uD800":1,"extension\uFFFDField"#, false),
            (r"rlmDepth\uFFFD", false),
            (r"\uFFFDparentSession", false),
            (r"parentSession\uFFFD", false),
        ] {
            let file = session_file(&format!(
                "{{\"type\":\"session\",\"version\":3,\"id\":\"child\",\"cwd\":\"/tmp/project\",\"parentSession\":\"/tmp/parent.jsonl\",\"rlmDepth\":1,\"{encoded_key}\":1}}\n{{\"type\":\"message\",\"id\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{{\"input\":10,\"output\":5}}}}}}"
            ));
            let (messages, accounting) = parse_prime_agent_file_with_accounting(file.path());
            assert_eq!(messages.is_empty(), rejected, "escaped key {encoded_key:?}");
            assert_eq!(
                accounting.child_parent_path.is_none(),
                rejected,
                "the shared parser and accounting analyzer must agree for {encoded_key:?}"
            );
        }
        for damaged_key in ["rlmDep�th", "parentSess�on"] {
            let file = session_file(&format!(
                "{{\"type\":\"session\",\"version\":3,\"id\":\"child\",\"cwd\":\"/tmp/project\",\"parentSession\":\"/tmp/parent.jsonl\",\"rlmDepth\":1,\"{damaged_key}\":1}}\n{{\"type\":\"message\",\"id\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{{\"input\":10,\"output\":5}}}}}}"
            ));
            let (messages, accounting) = parse_prime_agent_file_with_accounting(file.path());
            assert!(messages.is_empty(), "literal damaged key {damaged_key:?}");
            assert!(
                accounting.child_parent_path.is_none(),
                "literal damaged key {damaged_key:?}"
            );
        }

        for extension_key in ["rlmDepth�", "�parentSession", "parentSession�"] {
            let file = session_file(&format!(
                "{{\"type\":\"session\",\"version\":3,\"id\":\"root\",\"cwd\":\"/tmp/project\",\"{extension_key}\":1}}\n{{\"type\":\"message\",\"id\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{{\"input\":10,\"output\":5}}}}}}"
            ));
            assert_eq!(
                parse_prime_agent_file(file.path()).len(),
                1,
                "valid UTF-8 extension key {extension_key:?}"
            );
        }
    }

    #[test]
    fn damaged_message_usage_does_not_advance_accounting_alignment() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            r#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project"}
{"type":"message","id":"assistant-damaged","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-damaged","usage":{"in�put":999,"output":1}}}
{"type":"message","id":"assistant-valid","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-valid","usage":{"input":20,"output":8}}}
{"type":"child_usage_attributed","id":"usage-valid","targetId":"assistant-valid","childUsage":{"input":3,"output":2},"aggregateUsage":{"input":20,"output":8}}
"#.as_bytes(),
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_agent_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("prime-agent:response:response-valid")
        );

        let accounting = analyze_prime_agent_accounting(file.path(), &messages);
        assert_eq!(accounting.adjustments.len(), 1);
        assert_eq!(
            accounting.adjustments[0].dedup_key,
            "prime-agent:response:response-valid"
        );
        assert_eq!(accounting.adjustments[0].attributions[0].id, "usage-valid");
    }

    #[test]
    fn skips_replacement_mangled_prime_join_keys() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            br#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project"}
"#,
        )
        .unwrap();
        file.write_all(
            b"{\"type\":\"message\",\"id\":\"assistant-\xff\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":20,\"output\":8}}}\n",
        )
        .unwrap();
        file.write_all(
            b"{\"type\":\"child_usage_attributed\",\"id\":\"usage-clean\",\"targetId\":\"assistant-\xff\",\"childUsage\":{\"input\":3,\"output\":2},\"aggregateUsage\":{\"input\":20,\"output\":8}}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_agent_file(file.path());
        let accounting = analyze_prime_agent_accounting(file.path(), &messages);

        assert_eq!(messages.len(), 1);
        assert!(messages[0]
            .dedup_key
            .as_deref()
            .is_some_and(|key| key.starts_with("prime-agent:damaged:")));
        assert!(accounting.attributions.is_empty());
        assert!(accounting.adjustments.is_empty());
    }

    #[test]
    fn skips_mangled_attribution_ids_even_with_clean_message_joins() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            br#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project"}
"#,
        )
        .unwrap();
        file.write_all(
            br#"{"type":"message","id":"assistant-clean","timestamp":"2026-08-08T00:00:01Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-clean","usage":{"input":20,"output":8}}}
"#,
        )
        .unwrap();
        file.write_all(
            b"{\"type\":\"child_usage_attributed\",\"id\":\"usage-\xff\",\"targetId\":\"assistant-clean\",\"childUsage\":{\"input\":3,\"output\":2},\"aggregateUsage\":{\"input\":20,\"output\":8}}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_agent_file(file.path());
        let accounting = analyze_prime_agent_accounting(file.path(), &messages);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("prime-agent:response:response-clean")
        );
        assert!(accounting.attributions.is_empty());
        assert!(accounting.adjustments.is_empty());
    }

    #[test]
    fn skips_attributions_with_damaged_nested_usage_keys() {
        for damaged_key in ["out�put", "cache�Read"] {
            let mut file = NamedTempFile::new().unwrap();
            file.write_all(
                br#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project"}
"#,
            )
            .unwrap();
            file.write_all(
                br#"{"type":"message","id":"assistant-clean","timestamp":"2026-08-08T00:00:01Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-clean","usage":{"input":100,"output":8}}}
"#,
            )
            .unwrap();
            writeln!(
                file,
                "{{\"type\":\"child_usage_attributed\",\"id\":\"usage-clean\",\"targetId\":\"assistant-clean\",\"childUsage\":{{\"input\":50,\"{damaged_key}\":999}},\"aggregateUsage\":{{\"input\":100,\"output\":8}}}}"
            )
            .unwrap();
            file.flush().unwrap();

            let messages = parse_prime_agent_file(file.path());
            let accounting = analyze_prime_agent_accounting(file.path(), &messages);

            assert_eq!(messages.len(), 1);
            assert!(accounting.attributions.is_empty());
            assert!(accounting.adjustments.is_empty());
        }
    }

    #[test]
    fn skips_attributions_with_invalid_bytes_in_nested_usage_keys() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            br#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project"}
{"type":"message","id":"assistant-clean","timestamp":"2026-08-08T00:00:01Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-clean","usage":{"input":100,"output":8}}}
"#,
        )
        .unwrap();
        file.write_all(
            b"{\"type\":\"child_usage_attributed\",\"id\":\"usage-child\",\"targetId\":\"assistant-clean\",\"childUsage\":{\"input\":50,\"out\xffput\":999},\"aggregateUsage\":{\"input\":100,\"output\":8}}\n",
        )
        .unwrap();
        file.write_all(
            b"{\"type\":\"child_usage_attributed\",\"id\":\"usage-aggregate\",\"targetId\":\"assistant-clean\",\"childUsage\":{\"input\":50,\"output\":0},\"aggregateUsage\":{\"in\xfeput\":100,\"output\":8}}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_agent_file(file.path());
        let accounting = analyze_prime_agent_accounting(file.path(), &messages);

        assert_eq!(messages.len(), 1);
        assert!(accounting.attributions.is_empty());
        assert!(accounting.adjustments.is_empty());
    }

    #[test]
    fn rejects_rlm_child_without_usable_parent_session() {
        for parent_field in ["", ",\"parentSession\":\"\""] {
            let file = session_file(&format!(
                "{{\"type\":\"session\",\"id\":\"child\",\"rlmDepth\":1{parent_field}}}\n{{\"type\":\"message\",\"id\":\"assistant\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{{\"input\":10}}}}}}"
            ));
            let (messages, accounting) = parse_prime_agent_file_with_accounting(file.path());
            assert!(messages.is_empty(), "parent field {parent_field:?}");
            assert!(accounting.child_message_usages.is_empty());
        }
    }

    #[test]
    fn rejects_mangled_parent_session_value() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            b"{\"type\":\"session\",\"version\":3,\"id\":\"child\",\"cwd\":\"/tmp/project\",\"parentSession\":\"/tmp/parent-\xff.jsonl\",\"rlmDepth\":1}\n",
        )
        .unwrap();
        file.write_all(
            br#"{"type":"message","id":"assistant-clean","timestamp":"2026-08-08T00:00:01Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-clean","usage":{"input":20,"output":8}}}
"#,
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_agent_file(file.path());
        let accounting = analyze_prime_agent_accounting(file.path(), &messages);

        assert!(messages.is_empty());
        assert!(accounting.child_parent_path.is_none());
        assert!(accounting.child_message_usages.is_empty());
    }

    #[test]
    fn sanitizes_replacement_mangled_model_and_provider() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            b"{\"type\":\"session\",\"version\":3,\"id\":\"root\",\"cwd\":\"/tmp/\xff/project\",\"rlmDepth\":0}\n",
        )
        .unwrap();
        file.write_all(b"{\"type\":\"session_info\",\"name\":\"agent-\xff\"}\n")
            .unwrap();
        file.write_all(
            b"{\"type\":\"message\",\"id\":\"assistant-clean\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"bad-\xff-provider\",\"model\":\"bad-\xff-model\",\"usage\":{\"input\":20,\"output\":8}}}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_agent_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "unknown");
        assert_eq!(messages[0].provider_id, "prime-agent");
        assert!(messages[0].workspace_key.is_none());
        assert!(messages[0].agent.is_none());
    }

    #[test]
    fn parses_root_session_without_counting_child_attribution_records() {
        let file = session_file(
            r#"{"type":"session","version":3,"id":"root-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"session_info","id":"info","parentId":null,"timestamp":"2026-08-08T00:00:00.500Z","name":"My renamed thread"}
{"type":"message","id":"assistant-1","parentId":"info","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"msg_provider_001","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}
{"type":"child_usage_attributed","id":"usage-1","parentId":"assistant-1","timestamp":"2026-08-08T00:00:02.000Z","targetId":"assistant-1","childUsage":{"input":500,"output":200,"cacheRead":0,"cacheWrite":0,"totalTokens":700},"aggregateUsage":{"input":600,"output":250,"cacheRead":20,"cacheWrite":10,"totalTokens":880},"origin":"spawn_task"}"#,
        );

        let messages = parse_prime_agent_file(file.path());

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "prime-agent");
        assert_eq!(message.session_id, "root-1");
        assert_eq!(message.workspace_key.as_deref(), Some("/tmp/project"));
        assert_eq!(message.tokens.input, 100);
        assert_eq!(message.tokens.output, 50);
        assert_eq!(message.tokens.cache_read, 20);
        assert_eq!(message.tokens.cache_write, 10);
        assert_eq!(message.agent, None, "a root thread name is not an agent");
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("prime-agent:response:msg_provider_001")
        );
    }

    #[test]
    fn attributes_rlm_child_messages_to_the_session_name() {
        let file = session_file(
            r#"{"type":"session","version":3,"id":"child-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","parentSession":"/tmp/root.jsonl","rlmDepth":1}
{"type":"session_info","id":"info","parentId":null,"timestamp":"2026-08-08T00:00:00.500Z","name":"api-reviewer"}
{"type":"message","id":"assistant-1","parentId":"info","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"openai","model":"gpt-5.4","usage":{"input":40,"output":12,"cacheRead":8,"cacheWrite":0,"totalTokens":60}}}"#,
        );

        let messages = parse_prime_agent_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent.as_deref(), Some("api-reviewer"));
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].model_id, "gpt-5.4");
    }

    #[test]
    fn keeps_aggregate_parent_when_the_attributed_child_is_unavailable() {
        let file = session_file(
            r#"{"type":"session","version":3,"id":"fork-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250}}}
{"type":"child_usage_attributed","id":"usage-1","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":50,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":70},"aggregateUsage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250},"origin":"spawn_task"}"#,
        );

        let messages = parse_prime_agent_file(file.path());
        let accounting = analyze_prime_agent_accounting(file.path(), &messages);
        let messages = reconcile_prime_agent_messages(messages, &[accounting]);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 150);
        assert_eq!(messages[0].tokens.output, 70);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.cache_write, 10);
    }

    #[test]
    fn unparseable_child_timestamp_does_not_shift_accounting_alignment() {
        // The streaming parser drops an RLM child's message when its timestamp
        // cannot be matched back to a parent attribution. The accounting-only
        // walk replays cached messages positionally, so it has to drop the same
        // record or every later index refers to the wrong message.
        let file = session_file(
            r#"{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","parentSession":"/tmp/parent.jsonl","rlmDepth":1}
{"type":"message","id":"assistant-unparseable","parentId":null,"timestamp":"2026-08-08 00:00:01","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-unparseable","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}
{"type":"message","id":"assistant-valid","parentId":null,"timestamp":"2026-08-08T00:00:02.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"response-valid","usage":{"input":20,"output":8,"cacheRead":0,"cacheWrite":0,"totalTokens":28}}}
{"type":"child_usage_attributed","id":"usage-valid","parentId":"assistant-valid","timestamp":"2026-08-08T00:00:03.000Z","targetId":"assistant-valid","childUsage":{"input":3,"output":2,"cacheRead":0,"cacheWrite":0,"totalTokens":5},"aggregateUsage":{"input":20,"output":8,"cacheRead":0,"cacheWrite":0,"totalTokens":28},"origin":"spawn_task"}"#,
        );

        let (messages, streamed) = parse_prime_agent_file_with_accounting(file.path());
        let replayed = analyze_prime_agent_accounting(file.path(), &messages);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("prime-agent:response:response-valid")
        );

        let adjustment_keys = |accounting: &PrimeFileAccounting| {
            accounting
                .adjustments
                .iter()
                .map(|adjustment| adjustment.dedup_key.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            adjustment_keys(&replayed),
            vec!["prime-agent:response:response-valid".to_string()]
        );
        assert_eq!(adjustment_keys(&replayed), adjustment_keys(&streamed));

        let child_joins = |accounting: &PrimeFileAccounting| {
            accounting
                .child_message_usages
                .iter()
                .map(|usage| (usage.timestamp, usage.usage.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(child_joins(&replayed), child_joins(&streamed));
        assert_eq!(
            child_joins(&replayed),
            vec![(
                parse_timestamp_str("2026-08-08T00:00:02.000Z"),
                TokenBreakdown {
                    input: 20,
                    output: 8,
                    ..TokenBreakdown::default()
                }
            )]
        );
    }

    #[test]
    fn blank_model_message_does_not_shift_accounting_alignment() {
        let file = session_file(
            r#"{"type":"session","version":3,"id":"root","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"blank","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"","responseId":"blank-response","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}
{"type":"message","id":"parent","parentId":"blank","timestamp":"2026-08-08T00:00:02.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250}}}
{"type":"child_usage_attributed","id":"usage-1","parentId":"parent","timestamp":"2026-08-08T00:00:03.000Z","targetId":"parent","childUsage":{"input":50,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":70},"aggregateUsage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250},"origin":"spawn_task"}"#,
        );

        let messages = parse_prime_agent_file(file.path());
        let accounting = analyze_prime_agent_accounting(file.path(), &messages);

        assert_eq!(messages.len(), 2);
        assert_eq!(accounting.adjustments.len(), 1);
        assert_eq!(
            accounting.adjustments[0].dedup_key,
            "prime-agent:response:parent-response"
        );
    }

    #[test]
    fn sibling_forks_preserve_each_distinct_unavailable_child_delta() {
        fn tokens(input: i64) -> TokenBreakdown {
            TokenBreakdown {
                input,
                ..TokenBreakdown::default()
            }
        }
        fn parent_message(input: i64, session: &str) -> UnifiedMessage {
            let mut message = UnifiedMessage::new(
                "prime-agent",
                "claude-opus-5",
                "anthropic",
                session,
                1,
                tokens(input),
                0.0,
            );
            message.dedup_key = Some("prime-agent:response:shared-parent".to_string());
            message
        }
        fn fork_accounting(
            source: &str,
            attribution_id: &str,
            child_input: i64,
        ) -> PrimeFileAccounting {
            let attribution = PrimeAttribution {
                id: attribution_id.to_string(),
                timestamp: Some(1),
                child_usage: tokens(child_input),
                aggregate_usage: tokens(100 + child_input),
            };
            PrimeFileAccounting {
                source_path: PathBuf::from(source),
                attributions: vec![attribution.clone()],
                adjustments: vec![PrimeUsageAdjustment {
                    dedup_key: "prime-agent:response:shared-parent".to_string(),
                    persisted_usage: tokens(100 + child_input),
                    attributions: vec![attribution],
                }],
                ..PrimeFileAccounting::default()
            }
        }

        let messages = vec![parent_message(150, "fork-a"), parent_message(130, "fork-b")];
        let accounting = [
            fork_accounting("fork-a.jsonl", "child-a", 50),
            fork_accounting("fork-b.jsonl", "child-b", 30),
        ];
        let messages = reconcile_prime_agent_messages(messages, &accounting);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 180);
    }

    #[test]
    fn equal_child_usage_is_matched_by_parent_lineage_and_completion_time() {
        let dir = tempfile::TempDir::new().unwrap();
        let parent_path = dir.path().join("parent.jsonl");
        let child_path = dir.path().join("child.jsonl");
        std::fs::write(
            &parent_path,
            r#"{"type":"session","version":3,"id":"parent","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent-a","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"model-a","responseId":"parent-response-a","usage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}
{"type":"child_usage_attributed","id":"usage-a","parentId":"parent-a","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent-a","childUsage":{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50},"aggregateUsage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150},"origin":"spawn_task"}
{"type":"message","id":"parent-b","parentId":"usage-a","timestamp":"2026-08-08T00:00:10.000Z","message":{"role":"assistant","provider":"anthropic","model":"model-b","responseId":"parent-response-b","usage":{"input":250,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":250}}}
{"type":"child_usage_attributed","id":"usage-b","parentId":"parent-b","timestamp":"2026-08-08T00:00:11.000Z","targetId":"parent-b","childUsage":{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50},"aggregateUsage":{"input":250,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":250},"origin":"spawn_task"}
"#,
        )
        .unwrap();
        std::fs::write(
            &child_path,
            format!(
                r#"{{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:10.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:11.001Z","message":{{"role":"assistant","provider":"anthropic","model":"child-model","responseId":"child-response","usage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}}}}}}
"#,
                serde_json::to_string(&parent_path.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();

        let parent_messages = parse_prime_agent_file(&parent_path);
        let child_messages = parse_prime_agent_file(&child_path);
        let accounting = [
            analyze_prime_agent_accounting(&parent_path, &parent_messages),
            analyze_prime_agent_accounting(&child_path, &child_messages),
        ];
        let messages = reconcile_prime_agent_messages(
            parent_messages.into_iter().chain(child_messages).collect(),
            &accounting,
        );

        let parent_a = messages
            .iter()
            .find(|message| {
                message.dedup_key.as_deref() == Some("prime-agent:response:parent-response-a")
            })
            .unwrap();
        let parent_b = messages
            .iter()
            .find(|message| {
                message.dedup_key.as_deref() == Some("prime-agent:response:parent-response-b")
            })
            .unwrap();
        assert_eq!(parent_a.tokens.input, 150);
        assert_eq!(parent_b.tokens.input, 200);
    }

    #[test]
    fn same_sized_child_from_another_parent_does_not_authorize_subtraction() {
        let dir = tempfile::TempDir::new().unwrap();
        let parent_path = dir.path().join("parent-a.jsonl");
        let child_path = dir.path().join("child-b.jsonl");
        let unrelated_parent = dir.path().join("parent-b.jsonl");
        std::fs::write(
            &parent_path,
            r#"{"type":"session","version":3,"id":"parent-a","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250}}}
{"type":"child_usage_attributed","id":"usage-a","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":50,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":70},"aggregateUsage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250},"origin":"spawn_task"}
"#,
        )
        .unwrap();
        std::fs::write(
            &child_path,
            format!(
                r#"{{"type":"session","version":3,"id":"child-b","timestamp":"2026-08-08T00:00:01.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child","parentId":null,"timestamp":"2026-08-08T00:00:02.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"child-response","usage":{{"input":50,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":70}}}}}}
"#,
                serde_json::to_string(&unrelated_parent.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();

        let parent_messages = parse_prime_agent_file(&parent_path);
        let child_messages = parse_prime_agent_file(&child_path);
        let accounting = [
            analyze_prime_agent_accounting(&parent_path, &parent_messages),
            analyze_prime_agent_accounting(&child_path, &child_messages),
        ];
        let messages = reconcile_prime_agent_messages(
            parent_messages.into_iter().chain(child_messages).collect(),
            &accounting,
        );

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.tokens.input)
                .sum::<i64>(),
            200
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.tokens.output)
                .sum::<i64>(),
            90
        );
    }

    #[test]
    fn copied_fork_history_keeps_a_cross_session_dedup_key() {
        let original = session_file(
            r#"{"type":"session","version":3,"id":"root-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"assistant-1","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"msg_provider_001","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}"#,
        );
        let fork = session_file(
            r#"{"type":"session","version":3,"id":"fork-2","timestamp":"2026-08-08T01:00:00.000Z","cwd":"/tmp/project","parentSession":"/tmp/root.jsonl","rlmDepth":0}
{"type":"message","id":"assistant-1","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"msg_provider_001","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}"#,
        );

        let original = parse_prime_agent_file(original.path());
        let fork = parse_prime_agent_file(fork.path());

        assert_eq!(original.len(), 1);
        assert_eq!(fork.len(), 1);
        assert_eq!(original[0].dedup_key, fork[0].dedup_key);
    }

    #[test]
    fn copied_fork_history_with_corrupt_id_deduplicates_once() {
        let mut original = NamedTempFile::new().unwrap();
        original
            .write_all(
                br#"{"type":"session","version":3,"id":"root-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
"#,
            )
            .unwrap();
        let message = b"{\"type\":\"message\",\"id\":\"assistant-\xff\",\"timestamp\":\"2026-08-08T00:00:01.000Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":100,\"output\":50}}}\n";
        original.write_all(message).unwrap();
        original.flush().unwrap();

        let mut fork = NamedTempFile::new().unwrap();
        fork.write_all(
            br#"{"type":"session","version":3,"id":"fork-2","timestamp":"2026-08-08T01:00:00.000Z","cwd":"/tmp/project","parentSession":"/tmp/root.jsonl","rlmDepth":0}
"#,
        )
        .unwrap();
        fork.write_all(message).unwrap();
        fork.flush().unwrap();

        let original_messages = parse_prime_agent_file(original.path());
        let fork_messages = parse_prime_agent_file(fork.path());

        assert_eq!(original_messages.len(), 1);
        assert_eq!(fork_messages.len(), 1);
        assert!(original_messages[0].dedup_key.is_some());
        assert_eq!(original_messages[0].dedup_key, fork_messages[0].dedup_key);
        let deduped = reconcile_prime_agent_messages(
            vec![original_messages[0].clone(), fork_messages[0].clone()],
            &[],
        );
        assert_eq!(deduped.len(), 1);
    }

    #[test]
    fn distinct_invalid_utf8_ids_keep_distinct_dedup_keys() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            br#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project"}
"#,
        )
        .unwrap();
        file.write_all(
            b"{\"type\":\"message\",\"id\":\"assistant-\xff\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":10,\"output\":5}}}\n",
        )
        .unwrap();
        file.write_all(
            b"{\"type\":\"message\",\"id\":\"assistant-\xfe\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":10,\"output\":5}}}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_agent_file(file.path());
        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].dedup_key, messages[1].dedup_key);

        let reconciled = reconcile_prime_agent_messages(messages, &[]);
        assert_eq!(reconciled.len(), 2);
        assert_eq!(
            reconciled
                .iter()
                .map(|message| message.tokens.input)
                .sum::<i64>(),
            20
        );
        assert_eq!(
            reconciled
                .iter()
                .map(|message| message.tokens.output)
                .sum::<i64>(),
            10
        );
    }

    #[test]
    fn copied_fork_history_without_response_or_event_timestamp_still_deduplicates() {
        let original = session_file(
            r#"{"type":"session","version":3,"id":"root-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"assistant-1","parentId":null,"message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        let fork = session_file(
            r#"{"type":"session","version":3,"id":"fork-2","timestamp":"2026-08-08T01:00:00.000Z","cwd":"/tmp/project","parentSession":"/tmp/root.jsonl","rlmDepth":0}
{"type":"message","id":"assistant-1","parentId":null,"message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}"#,
        );

        let original = parse_prime_agent_file(original.path());
        let fork = parse_prime_agent_file(fork.path());

        assert_ne!(original[0].timestamp, fork[0].timestamp);
        assert_eq!(original[0].dedup_key, fork[0].dedup_key);
    }

    /// Prime allocates attribution ids with `randomUUID().slice(0, 8)` and only
    /// collision-checks them against the current session's own id map, so the
    /// same 32-bit id can appear in two unrelated sessions. A collision must not
    /// let one lineage's parsed child authorize a subtraction in the other.
    #[test]
    fn colliding_attribution_ids_in_separate_lineages_stay_independent() {
        fn totals(reverse: bool) -> (i64, i64) {
            let dir = tempfile::TempDir::new().unwrap();
            let parent_a = dir.path().join("parent-a.jsonl");
            let child_a = dir.path().join("child-a.jsonl");
            let parent_b = dir.path().join("parent-b.jsonl");
            std::fs::write(
                &parent_a,
                r#"{"type":"session","version":3,"id":"parent-a","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-a-response","usage":{"input":120,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":120}}}
{"type":"child_usage_attributed","id":"deadbeef","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":20,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":20},"aggregateUsage":{"input":120,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":120},"origin":"spawn_task"}
"#,
            )
            .unwrap();
            std::fs::write(
                &child_a,
                format!(
                    r#"{{"type":"session","version":3,"id":"child-a","timestamp":"2026-08-08T00:00:01.500Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child","parentId":null,"timestamp":"2026-08-08T00:00:02.001Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"child-a-response","usage":{{"input":20,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":20}}}}}}
"#,
                    serde_json::to_string(&parent_a.to_string_lossy()).unwrap()
                ),
            )
            .unwrap();
            // Unrelated lineage reusing the same 8-hex attribution id. Its own
            // child transcript was pruned, so the aggregate parent must stand.
            std::fs::write(
                &parent_b,
                r#"{"type":"session","version":3,"id":"parent-b","timestamp":"2026-08-09T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-09T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-b-response","usage":{"input":130,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":130}}}
{"type":"child_usage_attributed","id":"deadbeef","parentId":"parent","timestamp":"2026-08-09T00:00:02.000Z","targetId":"parent","childUsage":{"input":30,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":30},"aggregateUsage":{"input":130,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":130},"origin":"spawn_task"}
"#,
            )
            .unwrap();

            let mut paths = vec![parent_a, child_a, parent_b];
            if reverse {
                paths.reverse();
            }
            let parsed: Vec<(PathBuf, Vec<UnifiedMessage>)> = paths
                .into_iter()
                .map(|path| {
                    let messages = parse_prime_agent_file(&path);
                    (path, messages)
                })
                .collect();
            let accounting: Vec<PrimeFileAccounting> = parsed
                .iter()
                .map(|(path, messages)| analyze_prime_agent_accounting(path, messages))
                .collect();
            let messages: Vec<UnifiedMessage> = parsed
                .into_iter()
                .flat_map(|(_, messages)| messages)
                .collect();
            let messages = reconcile_prime_agent_messages(messages, &accounting);

            let parent_b_input = messages
                .iter()
                .find(|message| {
                    message.dedup_key.as_deref() == Some("prime-agent:response:parent-b-response")
                })
                .map_or(0, |message| message.tokens.input);
            (
                messages.iter().map(|message| message.tokens.input).sum(),
                parent_b_input,
            )
        }

        // parent-a reconciles to 100, its parsed child contributes 20, and the
        // pruned lineage keeps its full aggregate 130.
        assert_eq!(totals(false), (250, 130));
        assert_eq!(totals(true), (250, 130));
    }

    #[test]
    fn attributed_child_larger_than_the_parent_aggregate_clamps_at_zero() {
        fn tokens(input: i64, output: i64) -> TokenBreakdown {
            TokenBreakdown {
                input,
                output,
                ..TokenBreakdown::default()
            }
        }

        let mut message = UnifiedMessage::new(
            "prime-agent",
            "claude-opus-5",
            "anthropic",
            "partial",
            1,
            tokens(40, 10),
            0.0,
        );
        message.dedup_key = Some("prime-agent:response:partial".to_string());
        let attribution = PrimeAttribution {
            id: "deadbeef".to_string(),
            timestamp: Some(1),
            child_usage: tokens(90, 25),
            aggregate_usage: tokens(40, 10),
        };
        let accounting = [PrimeFileAccounting {
            source_path: PathBuf::from("partial.jsonl"),
            attributions: vec![attribution.clone()],
            adjustments: vec![PrimeUsageAdjustment {
                dedup_key: "prime-agent:response:partial".to_string(),
                persisted_usage: tokens(40, 10),
                attributions: vec![attribution],
            }],
            ..PrimeFileAccounting::default()
        }];

        let messages = reconcile_prime_agent_messages(vec![message], &accounting);

        assert_eq!(messages.len(), 1);
        // 40 - 90 clamps to 0 instead of wrapping, then the unavailable child is
        // restored, so the row never reports a negative or absurd bucket.
        assert_eq!(messages[0].tokens.input, 90);
        assert_eq!(messages[0].tokens.output, 25);
    }

    /// Two child responses of the same size completing inside one timestamp
    /// millisecond used to produce two tied candidates for each attribution.
    /// Rejecting both ties left the parent aggregate holding both children while
    /// the two child transcripts were also counted, double counting them.
    #[test]
    fn concurrent_equal_sized_children_pair_off_with_their_attributions() {
        fn totals(reverse: bool) -> (i64, i64) {
            let dir = tempfile::TempDir::new().unwrap();
            let parent_path = dir.path().join("parent.jsonl");
            let child_a = dir.path().join("child-a.jsonl");
            let child_b = dir.path().join("child-b.jsonl");
            std::fs::write(
                &parent_path,
                r#"{"type":"session","version":3,"id":"parent","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":300,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":300}}}
{"type":"child_usage_attributed","id":"usage-a","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":100,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":100},"aggregateUsage":{"input":200,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":200},"origin":"spawn_task"}
{"type":"child_usage_attributed","id":"usage-b","parentId":"usage-a","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":100,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":100},"aggregateUsage":{"input":300,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":300},"origin":"spawn_task"}
"#,
            )
            .unwrap();
            // Both children answered the same parent in the same millisecond, so
            // neither timestamp distinguishes them from the other's attribution.
            for (path, response) in [
                (&child_a, "child-a-response"),
                (&child_b, "child-b-response"),
            ] {
                std::fs::write(
                    path,
                    format!(
                        r#"{{"type":"session","version":3,"id":"{response}","timestamp":"2026-08-08T00:00:01.500Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:02.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"{response}","usage":{{"input":100,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":100}}}}}}
"#,
                        serde_json::to_string(&parent_path.to_string_lossy()).unwrap()
                    ),
                )
                .unwrap();
            }

            let mut paths = vec![parent_path, child_a, child_b];
            if reverse {
                paths.reverse();
            }
            let parsed: Vec<(PathBuf, Vec<UnifiedMessage>)> = paths
                .into_iter()
                .map(|path| {
                    let messages = parse_prime_agent_file(&path);
                    (path, messages)
                })
                .collect();
            let accounting: Vec<PrimeFileAccounting> = parsed
                .iter()
                .map(|(path, messages)| analyze_prime_agent_accounting(path, messages))
                .collect();
            let messages: Vec<UnifiedMessage> = parsed
                .into_iter()
                .flat_map(|(_, messages)| messages)
                .collect();
            let messages = reconcile_prime_agent_messages(messages, &accounting);

            let parent_input = messages
                .iter()
                .find(|message| {
                    message.dedup_key.as_deref() == Some("prime-agent:response:parent-response")
                })
                .map_or(0, |message| message.tokens.input);
            (
                messages.iter().map(|message| message.tokens.input).sum(),
                parent_input,
            )
        }

        // The parent keeps only its own 100; each of the two 100-token children
        // is counted once from its own transcript.
        assert_eq!(totals(false), (300, 100));
        assert_eq!(totals(true), (300, 100));
    }

    /// A surviving child response with no completion timestamp cannot prove it
    /// is the child a timed attribution describes. Accepting it because it was
    /// the only same-sized bucket let an unrelated sibling authorize the
    /// subtraction of a pruned child, undercounting billable usage.
    #[test]
    fn an_untimed_sibling_child_does_not_authorize_a_pruned_child_subtraction() {
        fn totals(reverse: bool) -> (i64, i64) {
            let dir = tempfile::TempDir::new().unwrap();
            let parent_path = dir.path().join("parent.jsonl");
            let sibling_path = dir.path().join("sibling.jsonl");
            // The attributed child transcript is gone; only the attribution
            // records that it spent 50 input tokens inside the 150 aggregate.
            std::fs::write(
                &parent_path,
                r#"{"type":"session","version":3,"id":"parent","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}
{"type":"child_usage_attributed","id":"usage-a","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50},"aggregateUsage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150},"origin":"spawn_task"}
"#,
            )
            .unwrap();
            // An unrelated child of the same parent that happens to have spent
            // the same 50 input tokens, and whose entry carries no timestamp.
            std::fs::write(
                &sibling_path,
                format!(
                    r#"{{"type":"session","version":3,"id":"sibling","timestamp":"2026-08-08T00:00:20.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"sibling-response","usage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}}}}}}
"#,
                    serde_json::to_string(&parent_path.to_string_lossy()).unwrap()
                ),
            )
            .unwrap();

            let mut paths = vec![parent_path, sibling_path];
            if reverse {
                paths.reverse();
            }
            let parsed: Vec<(PathBuf, Vec<UnifiedMessage>)> = paths
                .into_iter()
                .map(|path| {
                    let messages = parse_prime_agent_file(&path);
                    (path, messages)
                })
                .collect();
            let accounting: Vec<PrimeFileAccounting> = parsed
                .iter()
                .map(|(path, messages)| analyze_prime_agent_accounting(path, messages))
                .collect();
            let messages: Vec<UnifiedMessage> = parsed
                .into_iter()
                .flat_map(|(_, messages)| messages)
                .collect();
            let messages = reconcile_prime_agent_messages(messages, &accounting);

            let parent_input = messages
                .iter()
                .find(|message| {
                    message.dedup_key.as_deref() == Some("prime-agent:response:parent-response")
                })
                .map_or(0, |message| message.tokens.input);
            (
                messages.iter().map(|message| message.tokens.input).sum(),
                parent_input,
            )
        }

        // The aggregate parent keeps its full 150 because the child it names was
        // never parsed, and the untimed sibling adds its own 50 on top.
        assert_eq!(totals(false), (200, 150));
        assert_eq!(totals(true), (200, 150));
    }

    /// Transcripts written before Prime timestamped its entries carry no timing
    /// on either side of the pair. Lineage plus usage is then the only identity
    /// that exists, so it must still authorize the subtraction rather than
    /// double counting every legacy child.
    #[test]
    fn timestampless_transcripts_still_match_on_lineage_and_usage() {
        fn totals(reverse: bool) -> (i64, i64) {
            let dir = tempfile::TempDir::new().unwrap();
            let parent_path = dir.path().join("parent.jsonl");
            let child_path = dir.path().join("child.jsonl");
            std::fs::write(
                &parent_path,
                r#"{"type":"session","version":3,"id":"parent","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}
{"type":"child_usage_attributed","id":"usage-a","parentId":"parent","targetId":"parent","childUsage":{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50},"aggregateUsage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150},"origin":"spawn_task"}
"#,
            )
            .unwrap();
            std::fs::write(
                &child_path,
                format!(
                    r#"{{"type":"session","version":3,"id":"child","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"child-response","usage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}}}}}}
"#,
                    serde_json::to_string(&parent_path.to_string_lossy()).unwrap()
                ),
            )
            .unwrap();

            let mut paths = vec![parent_path, child_path];
            if reverse {
                paths.reverse();
            }
            let parsed: Vec<(PathBuf, Vec<UnifiedMessage>)> = paths
                .into_iter()
                .map(|path| {
                    let messages = parse_prime_agent_file(&path);
                    (path, messages)
                })
                .collect();
            let accounting: Vec<PrimeFileAccounting> = parsed
                .iter()
                .map(|(path, messages)| analyze_prime_agent_accounting(path, messages))
                .collect();
            let messages: Vec<UnifiedMessage> = parsed
                .into_iter()
                .flat_map(|(_, messages)| messages)
                .collect();
            let messages = reconcile_prime_agent_messages(messages, &accounting);

            let parent_input = messages
                .iter()
                .find(|message| {
                    message.dedup_key.as_deref() == Some("prime-agent:response:parent-response")
                })
                .map_or(0, |message| message.tokens.input);
            (
                messages.iter().map(|message| message.tokens.input).sum(),
                parent_input,
            )
        }

        assert_eq!(totals(false), (150, 100));
        assert_eq!(totals(true), (150, 100));
    }

    /// Maximum-cardinality matching alone leaves which attribution wins a
    /// contested child response up to the order attributions happen to be
    /// visited in, which is their random 8-hex id order. The global token total
    /// survives that, but the per-model rows do not, and pricing is applied per
    /// model after reconciliation -- so the cost of a pruned child lands on the
    /// wrong model. The nearer pairing must win regardless of id order.
    #[test]
    fn the_nearest_attribution_wins_a_contested_child_response() {
        fn per_model_input(reverse: bool, swap_ids: bool) -> HashMap<String, i64> {
            let (id_a, id_b) = if swap_ids {
                ("ffffffff", "00000000")
            } else {
                ("00000000", "ffffffff")
            };
            let dir = tempfile::TempDir::new().unwrap();
            let parent_path = dir.path().join("parent.jsonl");
            let child_path = dir.path().join("child.jsonl");
            // Two parent responses, each persisting a 150 aggregate that is 100
            // of its own plus one 50-token child. Only the second parent's child
            // transcript survives; the first parent's child was pruned.
            std::fs::write(
                &parent_path,
                format!(
                    r#"{{"type":"session","version":3,"id":"parent","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}}
{{"type":"message","id":"parent-a","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{{"role":"assistant","provider":"anthropic","model":"model-a","responseId":"parent-response-a","usage":{{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}}}}
{{"type":"child_usage_attributed","id":"{id_a}","parentId":"parent-a","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent-a","childUsage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}},"aggregateUsage":{{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}},"origin":"spawn_task"}}
{{"type":"message","id":"parent-b","parentId":"{id_a}","timestamp":"2026-08-08T00:00:01.500Z","message":{{"role":"assistant","provider":"anthropic","model":"model-b","responseId":"parent-response-b","usage":{{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}}}}
{{"type":"child_usage_attributed","id":"{id_b}","parentId":"parent-b","timestamp":"2026-08-08T00:00:02.002Z","targetId":"parent-b","childUsage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}},"aggregateUsage":{{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}},"origin":"spawn_task"}}
"#
                ),
            )
            .unwrap();
            // The surviving child completed in the same millisecond as the
            // second parent's attribution, and two milliseconds from the first
            // parent's -- inside the tolerance window for both.
            std::fs::write(
                &child_path,
                format!(
                    r#"{{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:01.600Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:02.002Z","message":{{"role":"assistant","provider":"anthropic","model":"child-model","responseId":"child-response","usage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}}}}}}
"#,
                    serde_json::to_string(&parent_path.to_string_lossy()).unwrap()
                ),
            )
            .unwrap();

            let mut paths = vec![parent_path, child_path];
            if reverse {
                paths.reverse();
            }
            let parsed: Vec<(PathBuf, Vec<UnifiedMessage>)> = paths
                .into_iter()
                .map(|path| {
                    let messages = parse_prime_agent_file(&path);
                    (path, messages)
                })
                .collect();
            let accounting: Vec<PrimeFileAccounting> = parsed
                .iter()
                .map(|(path, messages)| analyze_prime_agent_accounting(path, messages))
                .collect();
            let messages: Vec<UnifiedMessage> = parsed
                .into_iter()
                .flat_map(|(_, messages)| messages)
                .collect();
            let messages = reconcile_prime_agent_messages(messages, &accounting);

            let mut per_model: HashMap<String, i64> = HashMap::new();
            for message in &messages {
                *per_model.entry(message.model_id.clone()).or_default() += message.tokens.input;
            }
            per_model
        }

        for reverse in [false, true] {
            for swap_ids in [false, true] {
                let per_model = per_model_input(reverse, swap_ids);
                assert_eq!(
                    per_model.get("model-a").copied(),
                    Some(150),
                    "the parent whose child was pruned keeps its aggregate \
                     (reverse={reverse}, swap_ids={swap_ids})"
                );
                assert_eq!(
                    per_model.get("model-b").copied(),
                    Some(100),
                    "the parent whose child survived keeps only its own usage \
                     (reverse={reverse}, swap_ids={swap_ids})"
                );
                assert_eq!(per_model.get("child-model").copied(), Some(50));
                assert_eq!(per_model.values().sum::<i64>(), 300);
            }
        }
    }

    /// Two fork copies that name each other as fork parent describe one fork
    /// history, so their copies of one attribution must collapse. Resolving each
    /// copy to itself instead makes the pair look like two independent
    /// attributions, and the unavailable child's delta is restored once per copy.
    #[test]
    fn a_fork_parent_loop_collapses_onto_one_lineage() {
        fn totals(reverse: bool, with_child: bool) -> (i64, i64) {
            let dir = tempfile::TempDir::new().unwrap();
            let first_path = dir.path().join("fork-a.jsonl");
            let second_path = dir.path().join("fork-b.jsonl");
            let child_path = dir.path().join("child.jsonl");
            // Each copy names the other as its fork parent, and both carry the
            // same response, the same attribution id, and the same 150 aggregate
            // that is 100 of their own plus one 50-token child.
            for (path, fork_parent) in [(&first_path, &second_path), (&second_path, &first_path)] {
                std::fs::write(
                    path,
                    format!(
                        r#"{{"type":"session","version":3,"id":"fork","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":0}}
{{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"shared-parent","usage":{{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}}}}
{{"type":"child_usage_attributed","id":"aaaa1111","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}},"aggregateUsage":{{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}},"origin":"spawn_task"}}
"#,
                        serde_json::to_string(&fork_parent.to_string_lossy()).unwrap()
                    ),
                )
                .unwrap();
            }
            if with_child {
                std::fs::write(
                    &child_path,
                    format!(
                        r#"{{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:01.500Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:02.000Z","message":{{"role":"assistant","provider":"anthropic","model":"child-model","responseId":"child-response","usage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}}}}}}
"#,
                        serde_json::to_string(&first_path.to_string_lossy()).unwrap()
                    ),
                )
                .unwrap();
            }

            let mut paths = vec![first_path, second_path];
            if with_child {
                paths.push(child_path);
            }
            if reverse {
                paths.reverse();
            }
            let parsed: Vec<(PathBuf, Vec<UnifiedMessage>)> = paths
                .into_iter()
                .map(|path| {
                    let messages = parse_prime_agent_file(&path);
                    (path, messages)
                })
                .collect();
            let accounting: Vec<PrimeFileAccounting> = parsed
                .iter()
                .map(|(path, messages)| analyze_prime_agent_accounting(path, messages))
                .collect();
            let messages: Vec<UnifiedMessage> = parsed
                .into_iter()
                .flat_map(|(_, messages)| messages)
                .collect();
            let messages = reconcile_prime_agent_messages(messages, &accounting);

            let parent_input = messages
                .iter()
                .find(|message| {
                    message.dedup_key.as_deref() == Some("prime-agent:response:shared-parent")
                })
                .map_or(0, |message| message.tokens.input);
            (
                messages.iter().map(|message| message.tokens.input).sum(),
                parent_input,
            )
        }

        for reverse in [false, true] {
            // The child was never parsed, so the one aggregate is kept whole and
            // counted once rather than once per fork copy.
            assert_eq!(totals(reverse, false), (150, 150), "reverse={reverse}");
            // The child transcript is available, so the collapsed parent keeps
            // only its own 100 and the child is counted once from its own file.
            assert_eq!(totals(reverse, true), (150, 100), "reverse={reverse}");
        }
    }

    /// The partial case: three parent responses each claim a 50-token child
    /// inside one timestamp window, but only two of those child transcripts
    /// survive. Maximum cardinality fixes that two attributions are matched
    /// without saying which two, so the surviving transcripts must go to the
    /// attributions they are nearest to, not to whichever the scan reaches first.
    #[test]
    fn partial_equal_usage_matches_keep_each_attributions_identity() {
        fn per_model_input(reverse: bool, descending_ids: bool) -> HashMap<String, i64> {
            let mut ids = ["00000000", "88888888", "ffffffff"];
            if descending_ids {
                ids.reverse();
            }
            let dir = tempfile::TempDir::new().unwrap();
            let parent_path = dir.path().join("parent.jsonl");
            let mut parent = r#"{"type":"session","version":3,"id":"parent","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
"#
            .to_string();
            // model-a's child was pruned; model-b's and model-c's survived, each
            // completing in the same millisecond as its own attribution.
            for (model, id, millis) in [
                ("model-a", ids[0], "000"),
                ("model-b", ids[1], "003"),
                ("model-c", ids[2], "006"),
            ] {
                parent.push_str(&format!(
                    r#"{{"type":"message","id":"parent-{model}","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{{"role":"assistant","provider":"anthropic","model":"{model}","responseId":"response-{model}","usage":{{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}}}}
{{"type":"child_usage_attributed","id":"{id}","parentId":"parent-{model}","timestamp":"2026-08-08T00:00:02.{millis}Z","targetId":"parent-{model}","childUsage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}},"aggregateUsage":{{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}},"origin":"spawn_task"}}
"#
                ));
            }
            std::fs::write(&parent_path, parent).unwrap();

            let mut paths = vec![parent_path.clone()];
            for (name, millis) in [("child-b", "003"), ("child-c", "006")] {
                let child_path = dir.path().join(format!("{name}.jsonl"));
                std::fs::write(
                    &child_path,
                    format!(
                        r#"{{"type":"session","version":3,"id":"{name}","timestamp":"2026-08-08T00:00:01.500Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:02.{millis}Z","message":{{"role":"assistant","provider":"anthropic","model":"child-model","responseId":"{name}-response","usage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}}}}}}
"#,
                        serde_json::to_string(&parent_path.to_string_lossy()).unwrap()
                    ),
                )
                .unwrap();
                paths.push(child_path);
            }
            if reverse {
                paths.reverse();
            }

            let parsed: Vec<(PathBuf, Vec<UnifiedMessage>)> = paths
                .into_iter()
                .map(|path| {
                    let messages = parse_prime_agent_file(&path);
                    (path, messages)
                })
                .collect();
            let accounting: Vec<PrimeFileAccounting> = parsed
                .iter()
                .map(|(path, messages)| analyze_prime_agent_accounting(path, messages))
                .collect();
            let messages: Vec<UnifiedMessage> = parsed
                .into_iter()
                .flat_map(|(_, messages)| messages)
                .collect();
            let messages = reconcile_prime_agent_messages(messages, &accounting);

            let mut per_model: HashMap<String, i64> = HashMap::new();
            for message in &messages {
                *per_model.entry(message.model_id.clone()).or_default() += message.tokens.input;
            }
            per_model
        }

        for reverse in [false, true] {
            for descending_ids in [false, true] {
                let per_model = per_model_input(reverse, descending_ids);
                let context = format!("reverse={reverse}, descending_ids={descending_ids}");
                assert_eq!(
                    per_model.get("model-a").copied(),
                    Some(150),
                    "the pruned child's aggregate is retained ({context})"
                );
                assert_eq!(
                    per_model.get("model-b").copied(),
                    Some(100),
                    "model-b's own child authorizes its subtraction ({context})"
                );
                assert_eq!(
                    per_model.get("model-c").copied(),
                    Some(100),
                    "model-c's own child authorizes its subtraction ({context})"
                );
                assert_eq!(per_model.get("child-model").copied(), Some(100));
                assert_eq!(per_model.values().sum::<i64>(), 450);
            }
        }
    }

    /// The definition the fast derivation implements: re-solve the component
    /// with one attribution removed and see whether an equally cheap maximum
    /// matching survives without it.
    fn indispensable_by_reprobe(component: &MatchingComponent) -> Vec<bool> {
        let (cardinality, cost, assignment) = min_cost_max_matching(component, None);
        let saturated = cardinality == component.edges.len();
        assignment
            .iter()
            .enumerate()
            .map(|(local, matched)| {
                if matched.is_none() {
                    return false;
                }
                if saturated {
                    return true;
                }
                let (alternate_cardinality, alternate_cost, _) =
                    min_cost_max_matching(component, Some(local));
                !(alternate_cardinality == cardinality && alternate_cost == cost)
            })
            .collect()
    }

    /// Deterministic xorshift, so a failing case is reproducible from its seed
    /// without pulling a random-number crate into the dependency tree.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.next() % bound as u64) as usize
        }
    }

    fn random_component(rng: &mut Rng, attributions: usize, children: usize) -> MatchingComponent {
        // Three cost shapes, because ties are what make the tie rule bite: all
        // pairings equally close, a mix of exact and merely-in-window hits, and
        // fully spread timestamp distances.
        let shape = rng.below(3);
        let edges = (0..attributions)
            .map(|_| {
                let mut candidates: Vec<(usize, MatchCost)> = Vec::new();
                for child in 0..children {
                    // Partially pruned children: not every attribution reaches
                    // every child response.
                    if rng.below(3) == 0 {
                        continue;
                    }
                    let cost = match shape {
                        0 => [0, 0, 1001][rng.below(3)],
                        1 => [0, 1, 2, 1001][rng.below(4)],
                        _ => rng.below(1001) as MatchCost,
                    };
                    candidates.push((child, cost));
                }
                candidates.sort();
                candidates
            })
            .collect();
        MatchingComponent {
            attributions: (0..attributions).collect(),
            edges,
            children,
        }
    }

    #[test]
    fn one_matching_decides_indispensability_the_same_way_as_re_solving() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut checked = 0usize;
        for _ in 0..4_000 {
            let attributions = 1 + rng.below(8);
            let children = 1 + rng.below(8);
            let component = random_component(&mut rng, attributions, children);
            if component.edges.iter().all(|edges| edges.is_empty()) {
                continue;
            }
            let (_, _, assignment) = min_cost_max_matching(&component, None);
            assert_eq!(
                indispensable_attributions(&component, &assignment),
                indispensable_by_reprobe(&component),
                "component {:?} with {children} children",
                component.edges
            );
            checked += 1;
        }
        assert!(checked > 3_000, "only {checked} components were generated");
    }

    #[test]
    fn a_large_equal_usage_component_with_pruned_children_resolves_quickly() {
        // 60 attributions in one equal-usage bucket, all landing on the same
        // completion timestamp, with only half the child responses still on
        // disk: the legacy shape where the per-attribution re-probe used to run
        // a full matching 30 times over.
        let attributions = 60usize;
        let children = attributions / 2;
        let component = MatchingComponent {
            attributions: (0..attributions).collect(),
            edges: (0..attributions)
                .map(|_| (0..children).map(|child| (child, 0)).collect())
                .collect(),
            children,
        };
        let start = std::time::Instant::now();
        let (cardinality, _, assignment) = min_cost_max_matching(&component, None);
        let indispensable = indispensable_attributions(&component, &assignment);
        let elapsed = start.elapsed();

        assert_eq!(cardinality, children);
        // Every attribution is interchangeable, so no matching is forced and
        // each parent aggregate is kept.
        assert!(indispensable.iter().all(|forced| !forced));
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "matching a 60-attribution component took {elapsed:?}"
        );
    }

    #[test]
    fn rejects_the_rlm_subagent_catalog_as_a_session() {
        let file = session_file(
            r#"{"type":"rlm_subagent","childId":"sub-deadbeef","sessionName":"worker","sessionFile":"/tmp/child.jsonl"}"#,
        );

        assert!(parse_prime_agent_file(file.path()).is_empty());
    }
}
