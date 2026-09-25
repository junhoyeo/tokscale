use serde::{Deserialize, Serialize};

/// Supported TUI display languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TuiLanguage {
    #[default]
    #[serde(rename = "en")]
    En,
    #[serde(rename = "ko")]
    Ko,
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "fr")]
    Fr,
}

impl TuiLanguage {
    pub const ALL: [TuiLanguage; 5] = [
        TuiLanguage::En,
        TuiLanguage::Ko,
        TuiLanguage::Ja,
        TuiLanguage::ZhCn,
        TuiLanguage::Fr,
    ];

    /// Canonical code, e.g. "en", "ko", "zh-CN".
    pub const fn code(&self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Ko => "ko",
            Self::Ja => "ja",
            Self::ZhCn => "zh-CN",
            Self::Fr => "fr",
        }
    }

    /// Autonym (native display name), e.g. "한국어", "日本語".
    pub const fn native_name(&self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Ko => "한국어",
            Self::Ja => "日本語",
            Self::ZhCn => "简体中文",
            Self::Fr => "Français",
        }
    }

    #[allow(dead_code)]
    pub fn from_code(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "en" | "english" => Some(Self::En),
            "ko" | "korean" | "한국어" => Some(Self::Ko),
            "ja" | "japanese" | "日本語" => Some(Self::Ja),
            "zh-cn" | "zh_cn" | "zh" | "chinese" | "简体中文" => Some(Self::ZhCn),
            "fr" | "french" | "français" | "francais" => Some(Self::Fr),
            _ => None,
        }
    }
}

/// Identifiers for localizable UI text in Tokscale TUI.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageKey {
    // Tabs
    TabOverview,
    TabUsage,
    TabModels,
    TabDaily,
    TabHourly,
    TabMinutely,
    TabMonthly,
    TabSessions,
    TabProjects,
    TabStats,
    TabAgents,

    // Short tab names (for very narrow terminals)
    TabOverviewShort,
    TabUsageShort,
    TabModelsShort,
    TabDailyShort,
    TabHourlyShort,
    TabMinutelyShort,
    TabMonthlyShort,
    TabSessionsShort,
    TabProjectsShort,
    TabStatsShort,
    TabAgentsShort,

    ColDate,
    ColCost,
    ColTokens,
    ColModel,
    ColClient,
    /// `ColClient` for the percentage-width narrow tables, where the full word
    /// does not fit in CJK. See `tr_ja`'s note.
    ColClientShort,
    ColProvider,
    ColSource,
    ColMessages,
    ColMessagesShort,
    ColInput,
    ColOutput,
    ColCacheRead,
    ColCacheWrite,
    ColSession,
    ColProject,
    ColDuration,
    ColWorkspace,
    ColTotal,
    ColTurn,
    ColMonth,
    ColHour,
    ColMinute,
    ColAgent,
    ColSessions,
    ColSources,
    ColModels,
    ColRank,
    ColMsPer1k,
    ColCostPer1M,
    ColLastActive,

    // Overview Panel
    OverviewTopModels,
    OverviewModelsByTokens,
    OverviewModelsByCost,
    OverviewTotal,

    // Empty state messages
    EmptyNoUsageData,
    EmptyNoModelDetailsDay,

    // Footer counts & labels
    FooterTokens,
    CountModels,
    CountAgents,
    CountDay,
    CountDays,
    CountHours,
    CountMinutes,
    CountMonths,
    CountSessions,
    CountProjects,

    // Sort buttons
    SortLabel,
    SortDate,
    SortCost,
    SortTokens,

    // Navigation and Help Hints
    HelpScroll,
    HelpSort,
    HelpBack,
    HelpDetails,
    HelpToday,
    HelpProfile,
    HelpSources,
    HelpLanguage,
    HelpRefresh,
    HelpQuit,

    // Dialogs: Common
    DialogCloseHint,
    DialogFilterPlaceholder,
    DialogFilterLabel,
    DialogNoResults,

    // Dialog: Language Picker
    LanguageDialogTitle,
    LanguageDialogHint,

    // Dialog: Client Picker
    ClientDialogTitle,
    ClientDialogHint,

    // Dialog: Group By
    GroupByDialogTitle,
    GroupByDialogHint,
    GroupByModelLabel,
    GroupByModelDesc,
    GroupByClientModelLabel,
    GroupByClientModelDesc,
    GroupByClientProviderModelLabel,
    GroupByClientProviderModelDesc,
    GroupByWorkspaceModelLabel,
    GroupByWorkspaceModelDesc,
    GroupBySessionLabel,
    GroupBySessionDesc,
    GroupByClientSessionLabel,
    GroupByClientSessionDesc,

    // Dialog: Confirm
    ConfirmYes,
    ConfirmNo,

    // Status / Messages
    StatusLoadedFromCache,
    StatusRefreshInProgress,
    StatusLanguageChanged,

    ColCacheHit,

    // Block & Panel Titles
    TitleDailyUsage,
    TitleMonthlyUsage,
    TitleHourlyUsage,
    TitleHourlyProfile,
    TitleMinutelyUsage,
    TitleUsageSummary,
    TitleAccounts,
    TitleSelectedAccount,
    TitleContributionGraph,
    TitleDayBreakdown,
    TitleDailyDetail,
    TitleDailyDetailPrefix,
    TitleDailyBreakdown,
    TitleDailyBreakdownPrefix,

    // Chart Titles
    ChartTokensPerDay,
    ChartTokens,

    // Empty state messages (additional)
    EmptyNoDailyData,
    EmptyNoMonthlyData,
    EmptyNoDailyDataMonth,
    EmptyNoHourlyData,
    EmptyNoMinutelyData,
    EmptyNoSessionData,
    EmptyNoProjectData,
    EmptyNoDataAvailable,
    EmptyNoDataForDay,
    EmptyNoBreakdownAvailable,
    EmptyNoAgentCodex,
    EmptyNoAgentMixed,

    // Footer & Status (additional)
    StatusRefreshingBackground,
    StatusLastUpdated,
    StatusLocal,
    StatusDevice,
    StatusAllDevices,

    // Stats Panel Labels
    StatsFavoriteModel,
    StatsFavoriteModelShort,
    StatsTotalTokens,
    StatsTokensShort,
    StatsSessions,
    StatsTotalCost,
    StatsCostShort,
    StatsCurrentStreak,
    StatsStreakShort,
    StatsLongestStreak,
    StatsLongestStreakShort,
    StatsActiveDays,
    StatsActiveShort,
    StatsLess,
    StatsMore,

    // Dialog labels (additional)
    DialogCurrentLabel,
    DialogCancel,
    DialogTargetLabel,
    DialogEffectLabel,

    // Loading / Phase messages
    PhaseInitializing,
    PhaseScanningSessions,
    PhaseLoadingPricing,
    PhaseFinalizingReport,
    PhaseComplete,
    PhaseLoadingData,
    LabelError,

    // Hourly Profile Labels
    ProfileWhenYouWorkMost,
    ProfileMostProductiveDay,
    ProfilePeakHour,
    ProfileLegend,
    ProfileLow,
    ProfileHigh,
    ProfileTotalTokens,
    ProfileTotalCost,
    PeriodMorning,
    PeriodDaytime,
    PeriodEvening,
    PeriodNight,
    WeekdayMonday,
    WeekdayTuesday,
    WeekdayWednesday,
    WeekdayThursday,
    WeekdayFriday,
    WeekdaySaturday,
    WeekdaySunday,
    ProfileSwitchHint,

    // Save failure
    StatusLanguageSaveFailed,

    // Usage status
    StatusSyncingUsage,
    StatusCodexLogin,
    StatusNoData,
    StatusNotLoaded,
    StatusSavedSingular,
    StatusSavedPlural,
    StatusManagedSingular,
    StatusManagedPlural,
    StatusIssueSingular,
    StatusIssuePlural,

    // Usage action bar
    ActionRefresh,
    ActionRefreshSyncing,
    ActionAddCodex,
    ActionAddingCodex,
    ActionShowEmails,
    ActionHideEmails,
    ActionReset,

    // Usage section headings
    HeadingAttention,
    HeadingDiagnostics,
    HeadingProviders,
    HeadingLimits,
    HeadingActions,
    HeadingCreditBank,

    // Usage KV labels
    LabelStatus,
    LabelEmail,
    LabelCredential,
    LabelCredits,
    LabelResetBank,

    // Usage accounts table
    ColAccount,
    ColPlan,
    ColAuth,
    ColHealth,
    ColLimit,
    ColReset,
    UsageAccountOrStatus,
    // Usage: empty and failure states
    UsageEmptyNoData,
    UsageEmptyNotLoadedTitle,
    UsageEmptyNotLoadedHint,
    UsageFetchFailed,
    UsageEmptyNoSubscriptionData,
    UsageNoAttentionNeeded,

    // Usage: overall state vocabulary (the `State` row's value)
    StateSwitchRecommended,
    StateReady,
    StateReadyWithWarnings,
    StateQuotaLow,
    StateUnknown,

    // Usage: per-account readiness, which also fills the `Health` column
    HealthReady,
    HealthWatch,
    HealthQuotaLow,
    HealthUnknown,

    // Usage: summary K/V row labels, padded to 12 cells by `push_kv_styled`
    LabelState,
    LabelActiveAccount,
    LabelCapacity,
    LabelFallback,
    LabelNextReset,
    LabelAction,

    // Usage: summary K/V row values
    UsageNoActiveAccount,
    UsageNoReadyFallback,
    UsageNoResetData,
    UsagePercentLeft,
    CapacityReady,
    CapacityWatch,
    CapacityCritical,
    CapacityUnknown,

    // Usage: the recommended next action
    ActionChooseActive,
    ActionRefreshUnknownLimits,
    ActionKeepCurrent,
    ActionMonitorQuota,
    ActionUsePrefix,
    ActionWaitForReset,
    ActionRefreshActive,

    // Usage: account state, which also fills the `Auth` column
    AuthActive,
    AuthSaved,
    AuthManaged,
    StateAuthenticated,
    LabelUnknown,

    // Usage: "+N more ..." counters
    UsageMoreIssues,
    UsageMoreIssuesPlural,
    UsageMoreAtRisk,
    UsageMoreAtRiskPlural,

    // Usage: row-action buttons
    ButtonUseAccount,
    ButtonRemove,
    ButtonReset,

    // Usage: limits, metrics and the snapshot line
    UsageNoLimits,
    UsageNoQuotaMetrics,
    UsageNoQuotaMetricsReturned,
    UsageSnapshot,
    UsageAtRisk,
    UsageEmailsHidden,

    // Usage: credit-bank expiry rows and counters
    CreditExpiryUnknown,
    CreditExpiresPrefix,
    CreditNearestExpires,
    CreditMoreResetCredits,
    CreditMoreResetCreditsPlural,
    CreditCountSingular,
    CreditCountPlural,
    CreditAvailableSingular,
    CreditAvailablePlural,
    CreditAvailableAcrossAccounts,
    UsageNoResetCredits,

    // Usage: the Codex login panel
    CodexLoginTitle,
    CodexLoginImported,
    CodexLoginFailed,
    CodexLoginRunning,
    CodexLoginIdle,
    CodexLoginCancel,
    CodexLoginDismiss,
    CodexLoginWaiting,
    CodexLoginImportedPrefix,

    // Usage: the fetching spinner
    UsageFetchingShort,
    UsageFetchingLong,

    // Usage: compact (<48 columns) action-bar labels
    ActionRefreshSyncingShort,
    ActionAddingCodexShort,
    ActionAddCodexShort,
    ActionShowEmailsShort,
    ActionHideEmailsShort,

    // Usage: credential provenance wording
    CredentialSavedActive,
    CredentialSaved,
    CredentialManagedByPrefix,
    CredentialManagedExternally,
    UsageCurrentAccount,
    UsageManagedByPrefix,
    UsageManagedExternally,
}

pub fn tr(lang: TuiLanguage, key: MessageKey) -> &'static str {
    match lang {
        TuiLanguage::En => tr_en(key),
        TuiLanguage::Ko => tr_ko(key).unwrap_or_else(|| tr_en(key)),
        TuiLanguage::Ja => tr_ja(key).unwrap_or_else(|| tr_en(key)),
        TuiLanguage::ZhCn => tr_zh_cn(key).unwrap_or_else(|| tr_en(key)),
        TuiLanguage::Fr => tr_fr(key).unwrap_or_else(|| tr_en(key)),
    }
}

const fn tr_en(key: MessageKey) -> &'static str {
    match key {
        MessageKey::TabOverview => "Overview",
        MessageKey::TabUsage => "Usage",
        MessageKey::TabModels => "Models",
        MessageKey::TabDaily => "Daily",
        MessageKey::TabHourly => "Hourly",
        MessageKey::TabMinutely => "Minutely",
        MessageKey::TabMonthly => "Monthly",
        MessageKey::TabSessions => "Sessions",
        MessageKey::TabProjects => "Projects",
        MessageKey::TabStats => "Stats",
        MessageKey::TabAgents => "Agents",

        MessageKey::TabOverviewShort => "Ovw",
        MessageKey::TabUsageShort => "Use",
        MessageKey::TabModelsShort => "Mod",
        MessageKey::TabDailyShort => "Day",
        MessageKey::TabHourlyShort => "Hr",
        MessageKey::TabMinutelyShort => "Min",
        MessageKey::TabMonthlyShort => "Mon",
        MessageKey::TabSessionsShort => "Ses",
        MessageKey::TabProjectsShort => "Prj",
        MessageKey::TabStatsShort => "Sta",
        MessageKey::TabAgentsShort => "Agt",

        MessageKey::ColDate => "Date",
        MessageKey::ColCost => "Cost",
        MessageKey::ColTokens => "Tokens",
        MessageKey::ColModel => "Model",
        MessageKey::ColClient => "Client",
        MessageKey::ColClientShort => "Client",
        MessageKey::ColProvider => "Provider",
        MessageKey::ColSource => "Source",
        MessageKey::ColMessages => "Msgs",
        MessageKey::ColMessagesShort => "Msgs",
        MessageKey::ColInput => "Input",
        MessageKey::ColOutput => "Output",
        MessageKey::ColCacheRead => "Cache R",
        MessageKey::ColCacheWrite => "Cache W",
        MessageKey::ColSession => "Session",
        MessageKey::ColProject => "Project",
        MessageKey::ColDuration => "Duration",
        MessageKey::ColWorkspace => "Workspace",
        MessageKey::ColTotal => "Total",
        MessageKey::ColTurn => "Turn",
        MessageKey::ColMonth => "Month",
        MessageKey::ColHour => "Hour",
        MessageKey::ColMinute => "Minute",
        MessageKey::ColAgent => "Agent",
        MessageKey::ColSessions => "Sessions",
        MessageKey::ColSources => "Sources",
        MessageKey::ColModels => "Models",
        MessageKey::ColRank => "#",
        MessageKey::ColMsPer1k => "ms/1K",
        MessageKey::ColCostPer1M => "Cost/1M",
        MessageKey::ColLastActive => "Last Active",

        MessageKey::OverviewTopModels => "Top Models",
        MessageKey::OverviewModelsByTokens => "Models by Tokens",
        MessageKey::OverviewModelsByCost => "Models by Cost",
        MessageKey::OverviewTotal => "Total: ",

        MessageKey::EmptyNoUsageData => {
            "No usage data found. Press 'r' to refresh, 's' for sources, 'g' for grouping."
        }
        MessageKey::EmptyNoModelDetailsDay => {
            "No model details found for this day. Press Esc to go back."
        }

        MessageKey::FooterTokens => " tokens",
        MessageKey::CountModels => "models",
        MessageKey::CountAgents => "agents",
        MessageKey::CountDay => "day",
        MessageKey::CountDays => "days",
        MessageKey::CountHours => "hours",
        MessageKey::CountMinutes => "minutes",
        MessageKey::CountMonths => "months",
        MessageKey::CountSessions => "sessions",
        MessageKey::CountProjects => "projects",

        MessageKey::SortLabel => "Sort: ",
        MessageKey::SortDate => "Date",
        MessageKey::SortCost => "Cost",
        MessageKey::SortTokens => "Tokens",

        MessageKey::HelpScroll => "↑↓ scroll • ←→/tab view",
        MessageKey::HelpSort => "[d/t/c:sort]",
        MessageKey::HelpBack => "[esc:back]",
        MessageKey::HelpDetails => "[enter:details]",
        MessageKey::HelpToday => "[j:today]",
        MessageKey::HelpProfile => "[v:profile]",
        MessageKey::HelpSources => "[s:sources]",
        MessageKey::HelpLanguage => "[k:lang]",
        MessageKey::HelpRefresh => "[r:refresh]",
        MessageKey::HelpQuit => "q",

        MessageKey::DialogCloseHint => "Esc close",
        MessageKey::DialogFilterPlaceholder => "Type to filter...",
        MessageKey::DialogFilterLabel => "Filter: ",
        MessageKey::DialogNoResults => "No results",

        MessageKey::LanguageDialogTitle => " Select Language ",
        MessageKey::LanguageDialogHint => "↑↓ navigate • Enter select • Esc cancel",

        MessageKey::ClientDialogTitle => " Clients ",
        MessageKey::ClientDialogHint => "↑↓ navigate • Enter toggle • Esc close",

        MessageKey::GroupByDialogTitle => " Group By ",
        MessageKey::GroupByDialogHint => "↑↓ navigate • Enter select • Esc close",
        MessageKey::GroupByModelLabel => "Model",
        MessageKey::GroupByModelDesc => "One row per model (merge clients & providers)",
        MessageKey::GroupByClientModelLabel => "Client + Model",
        MessageKey::GroupByClientModelDesc => "One row per client-model pair (default)",
        MessageKey::GroupByClientProviderModelLabel => "Client + Provider + Model",
        MessageKey::GroupByClientProviderModelDesc => "Most granular — no merging",
        MessageKey::GroupByWorkspaceModelLabel => "Workspace + Model",
        MessageKey::GroupByWorkspaceModelDesc => "Group local usage by workspace key, then model",
        MessageKey::GroupBySessionLabel => "Session + Model",
        MessageKey::GroupBySessionDesc => {
            "One row per session_id and model (attribute cost per session)"
        }
        MessageKey::GroupByClientSessionLabel => "Client + Session + Model",
        MessageKey::GroupByClientSessionDesc => "One row per client, session_id, and model",

        MessageKey::ConfirmYes => "Yes",
        MessageKey::ConfirmNo => "No",

        MessageKey::StatusLoadedFromCache => "Loaded from cache",
        MessageKey::StatusRefreshInProgress => "Refresh already in progress",
        MessageKey::StatusLanguageChanged => "Language changed to",

        MessageKey::ColCacheHit => "Cache✕",

        MessageKey::TitleDailyUsage => " Daily Usage ",
        MessageKey::TitleMonthlyUsage => " Monthly Usage ",
        MessageKey::TitleHourlyUsage => " Hourly Usage ",
        MessageKey::TitleHourlyProfile => " Hourly Profile ",
        MessageKey::TitleMinutelyUsage => " Minutely Usage ",
        MessageKey::TitleUsageSummary => " Usage Summary ",
        MessageKey::TitleAccounts => " Accounts ",
        MessageKey::TitleSelectedAccount => " Selected Account ",
        MessageKey::TitleContributionGraph => " Contribution Graph (52 weeks) ",
        MessageKey::TitleDayBreakdown => " Day Breakdown (ESC to close) ",
        MessageKey::TitleDailyDetail => " Daily Detail ",
        MessageKey::TitleDailyDetailPrefix => " Daily Detail: ",
        MessageKey::TitleDailyBreakdown => " Daily Breakdown ",
        MessageKey::TitleDailyBreakdownPrefix => " Daily Breakdown: ",

        MessageKey::ChartTokensPerDay => "Tokens per Day",
        MessageKey::ChartTokens => "Tokens",

        MessageKey::EmptyNoDailyData => "No daily usage data found. Press 'r' to refresh.",
        MessageKey::EmptyNoMonthlyData => "No monthly usage data found. Press 'r' to refresh.",
        MessageKey::EmptyNoDailyDataMonth => "No daily data found for this month. Press Esc to go back.",
        MessageKey::EmptyNoHourlyData => "No hourly usage data found. Press 'r' to refresh.",
        MessageKey::EmptyNoMinutelyData => "No minutely usage data found. Press 'r' to refresh.",
        MessageKey::EmptyNoSessionData => "No session usage data found. Press 'r' to refresh.",
        MessageKey::EmptyNoProjectData => "No project usage data found. Press 'r' to refresh.",
        MessageKey::EmptyNoDataAvailable => "No data available",
        MessageKey::EmptyNoDataForDay => "No data for this day",
        MessageKey::EmptyNoBreakdownAvailable => "No detailed breakdown available",
        MessageKey::EmptyNoAgentCodex => "No agent breakdown is available for the current sources.\nThe selected source usually does not record agent metadata for regular sessions.\nPress 's' to try a different source.",
        MessageKey::EmptyNoAgentMixed => "No agent breakdown is available for the current sources.\nOnly some sources record agent metadata.\nPress 's' to change sources or 'r' to refresh.",

        MessageKey::StatusRefreshingBackground => "Refreshing cached data in background...",
        MessageKey::StatusLastUpdated => "Last updated: ",
        MessageKey::StatusLocal => "local",
        MessageKey::StatusDevice => "1 device",
        MessageKey::StatusAllDevices => " all devices: ",

        MessageKey::StatsFavoriteModel => "Favorite model:",
        MessageKey::StatsFavoriteModelShort => "Model:",
        MessageKey::StatsTotalTokens => "Total tokens:",
        MessageKey::StatsTokensShort => "Tokens:",
        MessageKey::StatsSessions => "Sessions:",
        MessageKey::StatsTotalCost => "Total cost:",
        MessageKey::StatsCostShort => "Cost:",
        MessageKey::StatsCurrentStreak => "Current streak:",
        MessageKey::StatsStreakShort => "Streak:",
        MessageKey::StatsLongestStreak => "Longest streak:",
        MessageKey::StatsLongestStreakShort => "Max streak:",
        MessageKey::StatsActiveDays => "Active days:",
        MessageKey::StatsActiveShort => "Active:",
        MessageKey::StatsLess => "Less",
        MessageKey::StatsMore => "More",

        MessageKey::DialogCurrentLabel => "Current: ",
        MessageKey::DialogCancel => "[ Cancel ]",
        MessageKey::DialogTargetLabel => "Target",
        MessageKey::DialogEffectLabel => "Effect",

        MessageKey::PhaseInitializing => "Initializing...",
        MessageKey::PhaseScanningSessions => "Scanning session data...",
        MessageKey::PhaseLoadingPricing => "Loading pricing data...",
        MessageKey::PhaseFinalizingReport => "Finalizing report...",
        MessageKey::PhaseComplete => "Complete",
        MessageKey::PhaseLoadingData => "Loading data...",
        MessageKey::LabelError => "Error",

        MessageKey::ProfileWhenYouWorkMost => "When You Work Most",
        MessageKey::ProfileMostProductiveDay => "Most Productive Day",
        MessageKey::ProfilePeakHour => "Peak Hour: ",
        MessageKey::ProfileLegend => "Legend: ",
        MessageKey::ProfileLow => "low",
        MessageKey::ProfileHigh => "high",
        MessageKey::ProfileTotalTokens => "total tokens",
        MessageKey::ProfileTotalCost => "total cost",
        MessageKey::PeriodMorning => "Morning",
        MessageKey::PeriodDaytime => "Daytime",
        MessageKey::PeriodEvening => "Evening",
        MessageKey::PeriodNight => "Night",
        MessageKey::WeekdayMonday => "Monday",
        MessageKey::WeekdayTuesday => "Tuesday",
        MessageKey::WeekdayWednesday => "Wednesday",
        MessageKey::WeekdayThursday => "Thursday",
        MessageKey::WeekdayFriday => "Friday",
        MessageKey::WeekdaySaturday => "Saturday",
        MessageKey::WeekdaySunday => "Sunday",
        MessageKey::ProfileSwitchHint => "Press [v] to switch to table view",

        MessageKey::StatusLanguageSaveFailed => "Failed to save language setting:",

        MessageKey::StatusSyncingUsage => "Syncing usage",
        MessageKey::StatusCodexLogin => "Codex login",
        MessageKey::StatusNoData => "No data",
        MessageKey::StatusNotLoaded => "Not loaded",
        MessageKey::StatusSavedSingular => "saved",
        MessageKey::StatusSavedPlural => "saved",
        MessageKey::StatusManagedSingular => "managed",
        MessageKey::StatusManagedPlural => "managed",
        MessageKey::StatusIssueSingular => "issue",
        MessageKey::StatusIssuePlural => "issues",

        MessageKey::ActionRefresh => "r Refresh",
        MessageKey::ActionRefreshSyncing => "r Syncing",
        MessageKey::ActionAddCodex => "a Add Codex",
        MessageKey::ActionAddingCodex => "a Adding Codex",
        MessageKey::ActionShowEmails => "m Show Emails",
        MessageKey::ActionHideEmails => "m Hide Emails",
        MessageKey::ActionReset => "x Reset",

        MessageKey::HeadingAttention => "Attention",
        MessageKey::HeadingDiagnostics => "Diagnostics",
        MessageKey::HeadingProviders => "Providers",
        MessageKey::HeadingLimits => "Limits",
        MessageKey::HeadingActions => "Actions",
        MessageKey::HeadingCreditBank => "Credit Bank",

        MessageKey::LabelStatus => "Status",
        MessageKey::LabelEmail => "Email",
        MessageKey::LabelCredential => "Credential",
        MessageKey::LabelCredits => "Credits",
        MessageKey::LabelResetBank => "Reset Bank",

        MessageKey::ColAccount => "Account",
        MessageKey::ColPlan => "Plan",
        MessageKey::ColAuth => "Auth",
        MessageKey::ColHealth => "Health",
        MessageKey::ColLimit => "Limit",
        MessageKey::ColReset => "Reset",
        MessageKey::UsageAccountOrStatus => "Account / Status",

        // Usage: empty and failure states
        MessageKey::UsageEmptyNoData => "No usage data",
        MessageKey::UsageEmptyNotLoadedTitle => "No subscription data loaded",
        MessageKey::UsageEmptyNotLoadedHint => "Use Refresh to sync provider usage, or Add Codex to save another account.",
        MessageKey::UsageFetchFailed => "Usage fetch failed",
        MessageKey::UsageEmptyNoSubscriptionData => "No subscription data available",
        MessageKey::UsageNoAttentionNeeded => "No accounts need attention",

        // Usage: overall state vocabulary
        MessageKey::StateSwitchRecommended => "Switch recommended",
        MessageKey::StateReady => "Ready",
        MessageKey::StateReadyWithWarnings => "Ready with warnings",
        MessageKey::StateQuotaLow => "Quota low",
        MessageKey::StateUnknown => "Unknown",

        // Usage: per-account readiness / the Health column
        MessageKey::HealthReady => "Ready",
        MessageKey::HealthWatch => "Watch",
        MessageKey::HealthQuotaLow => "Quota Low",
        MessageKey::HealthUnknown => "Unknown",

        // Usage: summary K/V row labels
        MessageKey::LabelState => "State",
        MessageKey::LabelActiveAccount => "Active",
        MessageKey::LabelCapacity => "Capacity",
        MessageKey::LabelFallback => "Fallback",
        MessageKey::LabelNextReset => "Next Reset",
        MessageKey::LabelAction => "Action",

        // Usage: summary K/V row values
        MessageKey::UsageNoActiveAccount => "No active account",
        MessageKey::UsageNoReadyFallback => "No ready fallback",
        MessageKey::UsageNoResetData => "No reset data",
        MessageKey::UsagePercentLeft => "% left",
        MessageKey::CapacityReady => "ready",
        MessageKey::CapacityWatch => "watch",
        MessageKey::CapacityCritical => "critical",
        MessageKey::CapacityUnknown => "unknown",

        // Usage: recommended next action
        MessageKey::ActionChooseActive => "Choose an active account",
        MessageKey::ActionRefreshUnknownLimits => "Refresh accounts with unknown limits",
        MessageKey::ActionKeepCurrent => "Keep current account",
        MessageKey::ActionMonitorQuota => "Monitor active quota",
        MessageKey::ActionUsePrefix => "Use",
        MessageKey::ActionWaitForReset => "Wait for reset or refresh",
        MessageKey::ActionRefreshActive => "Refresh active account",

        // Usage: account state / the Auth column
        MessageKey::AuthActive => "Active",
        MessageKey::AuthSaved => "Saved",
        MessageKey::AuthManaged => "Managed",
        MessageKey::StateAuthenticated => "Authenticated",
        MessageKey::LabelUnknown => "Unknown",

        // Usage: "+N more ..." counters
        MessageKey::UsageMoreIssues => "more issue",
        MessageKey::UsageMoreIssuesPlural => "more issues",
        MessageKey::UsageMoreAtRisk => "more at risk",
        MessageKey::UsageMoreAtRiskPlural => "more at risk",

        // Usage: row-action buttons
        MessageKey::ButtonUseAccount => "Use Account",
        MessageKey::ButtonRemove => "Remove",
        MessageKey::ButtonReset => "Reset",

        // Usage: limits, metrics and the snapshot line
        MessageKey::UsageNoLimits => "No limits",
        MessageKey::UsageNoQuotaMetrics => "No quota metrics",
        MessageKey::UsageNoQuotaMetricsReturned => "No quota metrics returned",
        MessageKey::UsageSnapshot => "Snapshot",
        MessageKey::UsageAtRisk => "at risk",
        MessageKey::UsageEmailsHidden => "emails hidden",

        // Usage: credit-bank expiry rows and counters
        MessageKey::CreditExpiryUnknown => "expiry unknown",
        MessageKey::CreditExpiresPrefix => "expires",
        MessageKey::CreditNearestExpires => "nearest expires",
        MessageKey::CreditMoreResetCredits => "more reset credit",
        MessageKey::CreditMoreResetCreditsPlural => "more reset credits",
        MessageKey::CreditCountSingular => "credit",
        MessageKey::CreditCountPlural => "credits",
        MessageKey::CreditAvailableSingular => "available",
        MessageKey::CreditAvailablePlural => "available",
        MessageKey::CreditAvailableAcrossAccounts => "available across accounts",
        MessageKey::UsageNoResetCredits => "No reset credits",

        // Usage: the Codex login panel
        MessageKey::CodexLoginTitle => "Codex Login",
        MessageKey::CodexLoginImported => "Imported",
        MessageKey::CodexLoginFailed => "Failed",
        MessageKey::CodexLoginRunning => "Running",
        MessageKey::CodexLoginIdle => "Idle",
        MessageKey::CodexLoginCancel => "[Cancel]",
        MessageKey::CodexLoginDismiss => "[Dismiss]",
        MessageKey::CodexLoginWaiting => "Waiting for codex output...",
        MessageKey::CodexLoginImportedPrefix => "Imported",

        // Usage: the fetching spinner
        MessageKey::UsageFetchingShort => "Fetching usage...",
        MessageKey::UsageFetchingLong => "Fetching subscription data...",

        // Usage: compact (<48 columns) action-bar labels
        MessageKey::ActionRefreshSyncingShort => "r Sync",
        MessageKey::ActionAddingCodexShort => "a Adding",
        MessageKey::ActionAddCodexShort => "a Add",
        MessageKey::ActionShowEmailsShort => "m Show",
        MessageKey::ActionHideEmailsShort => "m Hide",

        // Usage: credential provenance wording
        MessageKey::CredentialSavedActive => "saved store, current Codex login",
        MessageKey::CredentialSaved => "saved store",
        MessageKey::CredentialManagedByPrefix => "managed by",
        MessageKey::CredentialManagedExternally => "managed externally",
        MessageKey::UsageCurrentAccount => "Current account",
        MessageKey::UsageManagedByPrefix => "Managed by",
        MessageKey::UsageManagedExternally => "Managed externally",

    }
}

const fn tr_ko(key: MessageKey) -> Option<&'static str> {
    Some(match key {
        MessageKey::TabOverview => "개요",
        MessageKey::TabUsage => "사용량",
        MessageKey::TabModels => "모델",
        MessageKey::TabDaily => "일별",
        MessageKey::TabHourly => "시간별",
        MessageKey::TabMinutely => "분별",
        MessageKey::TabMonthly => "월별",
        MessageKey::TabSessions => "세션",
        MessageKey::TabProjects => "프로젝트",
        MessageKey::TabStats => "통계",
        MessageKey::TabAgents => "에이전트",

        MessageKey::TabOverviewShort => "개요",
        MessageKey::TabUsageShort => "사용",
        MessageKey::TabModelsShort => "모델",
        MessageKey::TabDailyShort => "일별",
        MessageKey::TabHourlyShort => "시별",
        MessageKey::TabMinutelyShort => "분별",
        MessageKey::TabMonthlyShort => "월별",
        MessageKey::TabSessionsShort => "세션",
        MessageKey::TabProjectsShort => "프로",
        MessageKey::TabStatsShort => "통계",
        MessageKey::TabAgentsShort => "에이",

        MessageKey::ColDate => "날짜",
        MessageKey::ColCost => "비용",
        MessageKey::ColTokens => "토큰",
        MessageKey::ColModel => "모델",
        MessageKey::ColClient => "클라이언트",
        // 4 cells. `클라이언트` is 10 and the narrow Sessions table grants this
        // column about 9, so it rendered as `클라이언`. `클라` is the ordinary
        // Korean clipping of `클라이언트` and keeps the wide and narrow layouts
        // reading as the same word; the column holds CLI *and* desktop clients
        // (`claude-code`, `codex`, `opencode`), so `도구` ("tool") named the
        // wrong thing and `CLI` would name only half of them.
        MessageKey::ColClientShort => "클라",
        MessageKey::ColProvider => "공급자",
        MessageKey::ColSource => "소스",
        MessageKey::ColMessages => "메시지",
        // 4 cells, because `SessionColumn::Msgs` budgets 5 when the wide
        // Sessions table also shows Turn and `메시지` is 6 — it rendered as
        // `메시`, a truncated word with no ellipsis. `건수` ("number of
        // items") is the count-column wording Korean tables use, and it
        // matches the Japanese short label `件数`.
        MessageKey::ColMessagesShort => "건수",
        MessageKey::ColInput => "입력",
        MessageKey::ColOutput => "출력",
        MessageKey::ColCacheRead => "캐시 읽기",
        MessageKey::ColCacheWrite => "캐시 쓰기",
        MessageKey::ColSession => "세션",
        MessageKey::ColProject => "프로젝트",
        MessageKey::ColDuration => "소요시간",
        MessageKey::ColWorkspace => "워크스페이스",
        MessageKey::ColTotal => "합계",
        MessageKey::ColTurn => "턴",
        MessageKey::ColMonth => "월",
        MessageKey::ColHour => "시간",
        MessageKey::ColMinute => "분",
        MessageKey::ColAgent => "에이전트",
        MessageKey::ColSessions => "세션",
        MessageKey::ColSources => "소스",
        MessageKey::ColModels => "모델",
        MessageKey::ColRank => "#",
        MessageKey::ColMsPer1k => "ms/1K",
        MessageKey::ColCostPer1M => "비용/1M",
        MessageKey::ColLastActive => "최근 활동",

        MessageKey::OverviewTopModels => "상위 모델",
        MessageKey::OverviewModelsByTokens => "토큰별 모델",
        MessageKey::OverviewModelsByCost => "비용별 모델",
        MessageKey::OverviewTotal => "총합: ",

        MessageKey::EmptyNoUsageData => {
            "사용량 데이터를 찾을 수 없습니다. 'r': 새로고침, 's': 소스, 'g': 그룹화."
        }
        MessageKey::EmptyNoModelDetailsDay => {
            "이 날짜의 모델 상세 내역이 없습니다. Esc를 눌러 돌아가기."
        }

        MessageKey::FooterTokens => " 토큰",
        MessageKey::CountModels => "개 모델",
        MessageKey::CountAgents => "개 에이전트",
        MessageKey::CountDay => "일",
        MessageKey::CountDays => "일",
        MessageKey::CountHours => "시간",
        MessageKey::CountMinutes => "분",
        MessageKey::CountMonths => "개월",
        MessageKey::CountSessions => "개 세션",
        MessageKey::CountProjects => "개 프로젝트",

        MessageKey::SortLabel => "정렬: ",
        MessageKey::SortDate => "날짜",
        MessageKey::SortCost => "비용",
        MessageKey::SortTokens => "토큰",

        MessageKey::HelpScroll => "↑↓ 스크롤 • ←→/tab 보기",
        MessageKey::HelpSort => "[d/t/c:정렬]",
        MessageKey::HelpBack => "[esc:뒤로]",
        MessageKey::HelpDetails => "[enter:상세]",
        MessageKey::HelpToday => "[j:오늘]",
        MessageKey::HelpProfile => "[v:프로필]",
        MessageKey::HelpSources => "[s:소스]",
        MessageKey::HelpLanguage => "[k:언어]",
        MessageKey::HelpRefresh => "[r:새로고침]",
        MessageKey::HelpQuit => "q",

        MessageKey::DialogCloseHint => "Esc 닫기",
        MessageKey::DialogFilterPlaceholder => "검색어 입력...",
        MessageKey::DialogFilterLabel => "필터: ",
        MessageKey::DialogNoResults => "결과 없음",

        MessageKey::LanguageDialogTitle => " 언어 선택 ",
        MessageKey::LanguageDialogHint => "↑↓ 탐색 • Enter 선택 • Esc 취소",

        MessageKey::ClientDialogTitle => " 클라이언트 ",
        MessageKey::ClientDialogHint => "↑↓ 탐색 • Enter 전환 • Esc 닫기",

        MessageKey::GroupByDialogTitle => " 그룹화 기준 ",
        MessageKey::GroupByDialogHint => "↑↓ 탐색 • Enter 선택 • Esc 닫기",
        MessageKey::GroupByModelLabel => "모델",
        MessageKey::GroupByModelDesc => "모델별 1개 행 (클라이언트 및 공급자 병합)",
        MessageKey::GroupByClientModelLabel => "클라이언트 + 모델",
        MessageKey::GroupByClientModelDesc => "클라이언트-모델 쌍별 1개 행 (기본값)",
        MessageKey::GroupByClientProviderModelLabel => "클라이언트 + 공급자 + 모델",
        MessageKey::GroupByClientProviderModelDesc => "가장 상세함 — 병합 없음",
        MessageKey::GroupByWorkspaceModelLabel => "워크스페이스 + 모델",
        MessageKey::GroupByWorkspaceModelDesc => "로컬 사용량을 워크스페이스 및 모델별로 그룹화",
        MessageKey::GroupBySessionLabel => "세션 + 모델",
        MessageKey::GroupBySessionDesc => "세션 ID 및 모델별 1개 행 (세션별 비용 산정)",
        MessageKey::GroupByClientSessionLabel => "클라이언트 + 세션 + 모델",
        MessageKey::GroupByClientSessionDesc => "클라이언트, 세션 ID 및 모델별 1개 행",

        MessageKey::ConfirmYes => "예",
        MessageKey::ConfirmNo => "아니오",

        MessageKey::StatusLoadedFromCache => "캐시에서 불러옴",
        MessageKey::StatusRefreshInProgress => "새로고침이 이미 진행 중입니다",
        MessageKey::StatusLanguageChanged => "언어가 다음으로 변경되었습니다:",

        MessageKey::ColCacheHit => "캐시✕",

        MessageKey::TitleDailyUsage => " 일별 사용량 ",
        MessageKey::TitleMonthlyUsage => " 월별 사용량 ",
        MessageKey::TitleHourlyUsage => " 시간별 사용량 ",
        MessageKey::TitleHourlyProfile => " 시간별 프로필 ",
        MessageKey::TitleMinutelyUsage => " 분별 사용량 ",
        MessageKey::TitleUsageSummary => " 사용량 요약 ",
        MessageKey::TitleAccounts => " 계정 목록 ",
        MessageKey::TitleSelectedAccount => " 선택된 계정 ",
        MessageKey::TitleContributionGraph => " 기여도 그래프 (52주) ",
        MessageKey::TitleDayBreakdown => " 일별 상세 (ESC: 닫기) ",
        MessageKey::TitleDailyDetail => " 일별 상세 ",
        MessageKey::TitleDailyDetailPrefix => " 일별 상세: ",
        MessageKey::TitleDailyBreakdown => " 일별 내역 ",
        MessageKey::TitleDailyBreakdownPrefix => " 일별 내역: ",

        MessageKey::ChartTokensPerDay => "일별 토큰",
        MessageKey::ChartTokens => "토큰",

        MessageKey::EmptyNoDailyData => "일별 사용량 데이터가 없습니다. 'r'을 눌러 새로고침하세요.",
        MessageKey::EmptyNoMonthlyData => "월별 사용량 데이터가 없습니다. 'r'을 눌러 새로고침하세요.",
        MessageKey::EmptyNoDailyDataMonth => "이 달의 일별 데이터가 없습니다. Esc를 눌러 돌아가기.",
        MessageKey::EmptyNoHourlyData => "시간별 사용량 데이터가 없습니다. 'r'을 눌러 새로고침하세요.",
        MessageKey::EmptyNoMinutelyData => "분별 사용량 데이터가 없습니다. 'r'을 눌러 새로고침하세요.",
        MessageKey::EmptyNoSessionData => "세션 사용량 데이터가 없습니다. 'r'을 눌러 새로고침하세요.",
        MessageKey::EmptyNoProjectData => "프로젝트 사용량 데이터가 없습니다. 'r'을 눌러 새로고침하세요.",
        MessageKey::EmptyNoDataAvailable => "데이터가 없습니다",
        MessageKey::EmptyNoDataForDay => "이 날의 데이터가 없습니다",
        MessageKey::EmptyNoBreakdownAvailable => "상세 내역이 없습니다",
        MessageKey::EmptyNoAgentCodex => "현재 소스에 대한 에이전트 내역을 사용할 수 없습니다.\n선택한 소스는 일반적으로 일반 세션의 에이전트 메타데이터를 기록하지 않습니다.\n's'를 눌러 다른 소스를 사용해 보세요.",
        MessageKey::EmptyNoAgentMixed => "현재 소스에 대한 에이전트 내역을 사용할 수 없습니다.\n일부 소스만 에이전트 메타데이터를 기록합니다.\n's'를 눌러 소스를 변경하거나 'r'을 눌러 새로고침하세요.",

        MessageKey::StatusRefreshingBackground => "백그라운드에서 캐시 데이터 새로고침 중...",
        MessageKey::StatusLastUpdated => "마지막 업데이트: ",
        MessageKey::StatusLocal => "로컬",
        MessageKey::StatusDevice => "1대 기기",
        MessageKey::StatusAllDevices => " 모든 기기: ",

        MessageKey::StatsFavoriteModel => "주 사용 모델:",
        MessageKey::StatsFavoriteModelShort => "모델:",
        MessageKey::StatsTotalTokens => "총 토큰:",
        MessageKey::StatsTokensShort => "토큰:",
        MessageKey::StatsSessions => "세션 수:",
        MessageKey::StatsTotalCost => "총 비용:",
        MessageKey::StatsCostShort => "비용:",
        MessageKey::StatsCurrentStreak => "현재 연속일:",
        MessageKey::StatsStreakShort => "연속:",
        MessageKey::StatsLongestStreak => "최장 연속일:",
        MessageKey::StatsLongestStreakShort => "최대 연속:",
        MessageKey::StatsActiveDays => "활동일:",
        MessageKey::StatsActiveShort => "활동:",
        MessageKey::StatsLess => "적음",
        MessageKey::StatsMore => "많음",

        MessageKey::DialogCurrentLabel => "현재: ",
        MessageKey::DialogCancel => "[ 취소 ]",
        MessageKey::DialogTargetLabel => "대상",
        MessageKey::DialogEffectLabel => "효과",

        MessageKey::PhaseInitializing => "초기화 중...",
        MessageKey::PhaseScanningSessions => "세션 데이터 스캔 중...",
        MessageKey::PhaseLoadingPricing => "가격 책정 데이터 로드 중...",
        MessageKey::PhaseFinalizingReport => "보고서 생성 완료 중...",
        MessageKey::PhaseComplete => "완료",
        MessageKey::PhaseLoadingData => "데이터 로드 중...",
        MessageKey::LabelError => "오류",

        MessageKey::ProfileWhenYouWorkMost => "주 작업 시간대",
        MessageKey::ProfileMostProductiveDay => "가장 생산적인 요일",
        MessageKey::ProfilePeakHour => "피크 시간: ",
        MessageKey::ProfileLegend => "범례: ",
        MessageKey::ProfileLow => "낮음",
        MessageKey::ProfileHigh => "높음",
        MessageKey::ProfileTotalTokens => "총 토큰",
        MessageKey::ProfileTotalCost => "총 비용",
        MessageKey::PeriodMorning => "아침",
        MessageKey::PeriodDaytime => "낮",
        MessageKey::PeriodEvening => "저녁",
        MessageKey::PeriodNight => "밤",
        MessageKey::WeekdayMonday => "월요일",
        MessageKey::WeekdayTuesday => "화요일",
        MessageKey::WeekdayWednesday => "수요일",
        MessageKey::WeekdayThursday => "목요일",
        MessageKey::WeekdayFriday => "금요일",
        MessageKey::WeekdaySaturday => "토요일",
        MessageKey::WeekdaySunday => "일요일",
        MessageKey::ProfileSwitchHint => "[v]: 테이블 보기로 전환",

        MessageKey::StatusLanguageSaveFailed => "언어 설정 저장 실패:",

        MessageKey::StatusSyncingUsage => "사용량 동기화 중",
        MessageKey::StatusCodexLogin => "Codex 로그인",
        MessageKey::StatusNoData => "데이터 없음",
        MessageKey::StatusNotLoaded => "로드되지 않음",
        MessageKey::StatusSavedSingular => "저장됨",
        MessageKey::StatusSavedPlural => "저장됨",
        MessageKey::StatusManagedSingular => "관리됨",
        MessageKey::StatusManagedPlural => "관리됨",
        MessageKey::StatusIssueSingular => "개 문제",
        MessageKey::StatusIssuePlural => "개 문제",

        MessageKey::ActionRefresh => "r 새로고침",
        MessageKey::ActionRefreshSyncing => "r 동기화 중",
        MessageKey::ActionAddCodex => "a Codex 추가",
        MessageKey::ActionAddingCodex => "a Codex 추가 중",
        MessageKey::ActionShowEmails => "m 이메일 표시",
        MessageKey::ActionHideEmails => "m 이메일 숨기기",
        MessageKey::ActionReset => "x 초기화",

        MessageKey::HeadingAttention => "주의",
        MessageKey::HeadingDiagnostics => "진단",
        MessageKey::HeadingProviders => "공급자",
        MessageKey::HeadingLimits => "한도",
        MessageKey::HeadingActions => "작업",
        MessageKey::HeadingCreditBank => "크레딧 보관함",

        MessageKey::LabelStatus => "상태",
        MessageKey::LabelEmail => "이메일",
        MessageKey::LabelCredential => "자격 증명",
        MessageKey::LabelCredits => "크레딧",
        // 8 cells. `초기화 보관함` is 13 and `push_kv_styled` pads its key to
        // 12, so the longer label pushed this row's value out of line with
        // every other row in the panel. `초기화권` is the voucher reading of a
        // reset credit.
        MessageKey::LabelResetBank => "초기화권",

        MessageKey::ColAccount => "계정",
        MessageKey::ColPlan => "요금제",
        MessageKey::ColAuth => "인증",
        MessageKey::ColHealth => "상태",
        MessageKey::ColLimit => "한도",
        MessageKey::ColReset => "초기화",
        MessageKey::UsageAccountOrStatus => "계정 / 상태",

        // Usage: empty and failure states
        MessageKey::UsageEmptyNoData => "사용량 데이터 없음",
        MessageKey::UsageEmptyNotLoadedTitle => "구독 데이터가 로드되지 않음",
        MessageKey::UsageEmptyNotLoadedHint => "새로고침으로 공급자 사용량을 동기화하거나, Codex 추가로 다른 계정을 저장하세요.",
        MessageKey::UsageFetchFailed => "사용량 조회 실패",
        MessageKey::UsageEmptyNoSubscriptionData => "사용 가능한 구독 데이터가 없습니다",
        MessageKey::UsageNoAttentionNeeded => "주의가 필요한 계정이 없습니다",

        // Usage: overall state vocabulary
        MessageKey::StateSwitchRecommended => "전환 권장",
        MessageKey::StateReady => "정상",
        MessageKey::StateReadyWithWarnings => "정상 (주의 있음)",
        MessageKey::StateQuotaLow => "한도 부족",
        // `미확인`, matching `HealthUnknown` and `CapacityUnknown`: the State
        // row and the Health column are visible on the same frame, so the same
        // concept must not be `알 수 없음` in one and `미확인` in the other.
        MessageKey::StateUnknown => "미확인",

        // Usage: per-account readiness / the Health column. The 8-cell column
        // forces short forms, so these must read as *deliberate* short forms of
        // the `State` row's words rather than as typos: `부족` is visibly the
        // clipped form of `한도 부족`, where the previous `한도부족` differed from
        // it only by a missing space.
        MessageKey::HealthReady => "정상",
        MessageKey::HealthWatch => "주의",
        MessageKey::HealthQuotaLow => "부족",
        MessageKey::HealthUnknown => "미확인",

        // Usage: summary K/V row labels
        MessageKey::LabelState => "상태",
        MessageKey::LabelActiveAccount => "활성 계정",
        MessageKey::LabelCapacity => "잔여 용량",
        MessageKey::LabelFallback => "대체 계정",
        MessageKey::LabelNextReset => "다음 초기화",
        MessageKey::LabelAction => "조치",

        // Usage: summary K/V row values
        MessageKey::UsageNoActiveAccount => "활성 계정 없음",
        MessageKey::UsageNoReadyFallback => "사용 가능한 대체 계정 없음",
        MessageKey::UsageNoResetData => "초기화 정보 없음",
        MessageKey::UsagePercentLeft => "% 남음",
        MessageKey::CapacityReady => "정상",
        MessageKey::CapacityWatch => "주의",
        MessageKey::CapacityCritical => "위험",
        MessageKey::CapacityUnknown => "미확인",

        // Usage: recommended next action
        MessageKey::ActionChooseActive => "활성 계정을 선택하세요",
        MessageKey::ActionRefreshUnknownLimits => "한도를 알 수 없는 계정을 새로고침하세요",
        MessageKey::ActionKeepCurrent => "현재 계정 유지",
        MessageKey::ActionMonitorQuota => "활성 계정 한도를 확인하세요",
        MessageKey::ActionUsePrefix => "전환:",
        MessageKey::ActionWaitForReset => "초기화를 기다리거나 새로고침하세요",
        MessageKey::ActionRefreshActive => "활성 계정을 새로고침하세요",

        // Usage: account state / the Auth column
        MessageKey::AuthActive => "활성",
        MessageKey::AuthSaved => "저장됨",
        MessageKey::AuthManaged => "관리됨",
        MessageKey::StateAuthenticated => "인증됨",
        // Same `Unknown` concept as `StateUnknown`/`HealthUnknown`, so the same
        // word.
        MessageKey::LabelUnknown => "미확인",

        // Usage: "+N more ..." counters. These are rendered as `{n}{word}`
        // (see `more_count_label`), so the Korean measure word belongs at the
        // head of the value: `+2건의 문제 더`, `+2개 위험 계정 더`.
        MessageKey::UsageMoreIssues => "건의 문제 더",
        MessageKey::UsageMoreIssuesPlural => "건의 문제 더",
        MessageKey::UsageMoreAtRisk => "개 위험 계정 더",
        MessageKey::UsageMoreAtRiskPlural => "개 위험 계정 더",

        // Usage: row-action buttons
        MessageKey::ButtonUseAccount => "이 계정 사용",
        MessageKey::ButtonRemove => "삭제",
        MessageKey::ButtonReset => "초기화",

        // Usage: limits, metrics and the snapshot line
        MessageKey::UsageNoLimits => "한도 정보 없음",
        MessageKey::UsageNoQuotaMetrics => "한도 지표 없음",
        MessageKey::UsageNoQuotaMetricsReturned => "한도 지표가 반환되지 않았습니다",
        MessageKey::UsageSnapshot => "요약",
        MessageKey::UsageAtRisk => "위험",
        MessageKey::UsageEmailsHidden => "이메일 숨김",

        // Usage: credit-bank expiry rows and counters
        MessageKey::CreditExpiryUnknown => "만료일 미확인",
        MessageKey::CreditExpiresPrefix => "만료",
        MessageKey::CreditNearestExpires => "가장 빠른 만료",
        MessageKey::CreditMoreResetCredits => "개 초기화권 더",
        MessageKey::CreditMoreResetCreditsPlural => "개 초기화권 더",
        MessageKey::CreditCountSingular => "개 초기화권",
        MessageKey::CreditCountPlural => "개 초기화권",
        MessageKey::CreditAvailableSingular => "개 사용 가능",
        MessageKey::CreditAvailablePlural => "개 사용 가능",
        MessageKey::CreditAvailableAcrossAccounts => "개 계정 전체 사용 가능",
        MessageKey::UsageNoResetCredits => "초기화권 없음",

        // Usage: the Codex login panel
        MessageKey::CodexLoginTitle => "Codex 로그인",
        MessageKey::CodexLoginImported => "가져옴",
        MessageKey::CodexLoginFailed => "실패",
        MessageKey::CodexLoginRunning => "진행 중",
        MessageKey::CodexLoginIdle => "대기",
        MessageKey::CodexLoginCancel => "[취소]",
        MessageKey::CodexLoginDismiss => "[닫기]",
        MessageKey::CodexLoginWaiting => "codex 출력을 기다리는 중...",
        MessageKey::CodexLoginImportedPrefix => "가져옴:",

        // Usage: the fetching spinner
        MessageKey::UsageFetchingShort => "사용량 조회 중...",
        MessageKey::UsageFetchingLong => "구독 데이터 조회 중...",

        // Usage: compact (<48 columns) action-bar labels
        MessageKey::ActionRefreshSyncingShort => "r 동기화",
        MessageKey::ActionAddingCodexShort => "a 추가 중",
        MessageKey::ActionAddCodexShort => "a 추가",
        MessageKey::ActionShowEmailsShort => "m 표시",
        MessageKey::ActionHideEmailsShort => "m 숨김",

        // Usage: credential provenance wording
        MessageKey::CredentialSavedActive => "저장소, 현재 Codex 로그인",
        MessageKey::CredentialSaved => "저장소",
        MessageKey::CredentialManagedByPrefix => "관리 주체:",
        MessageKey::CredentialManagedExternally => "외부에서 관리됨",
        MessageKey::UsageCurrentAccount => "현재 계정",
        MessageKey::UsageManagedByPrefix => "관리 주체:",
        MessageKey::UsageManagedExternally => "외부에서 관리됨",


    })
}

const fn tr_ja(key: MessageKey) -> Option<&'static str> {
    Some(match key {
        MessageKey::TabOverview => "概要",
        MessageKey::TabUsage => "使用量",
        MessageKey::TabModels => "モデル",
        MessageKey::TabDaily => "日別",
        MessageKey::TabHourly => "時間別",
        MessageKey::TabMinutely => "分別",
        MessageKey::TabMonthly => "月別",
        MessageKey::TabSessions => "セッション",
        MessageKey::TabProjects => "プロジェクト",
        MessageKey::TabStats => "統計",
        MessageKey::TabAgents => "エージェント",

        MessageKey::TabOverviewShort => "概要",
        MessageKey::TabUsageShort => "使用",
        MessageKey::TabModelsShort => "モデ",
        MessageKey::TabDailyShort => "日別",
        MessageKey::TabHourlyShort => "時別",
        MessageKey::TabMinutelyShort => "分別",
        MessageKey::TabMonthlyShort => "月別",
        MessageKey::TabSessionsShort => "セッ",
        MessageKey::TabProjectsShort => "プロ",
        MessageKey::TabStatsShort => "統計",
        MessageKey::TabAgentsShort => "エー",

        MessageKey::ColDate => "日付",
        MessageKey::ColCost => "コスト",
        MessageKey::ColTokens => "トークン",
        MessageKey::ColModel => "モデル",
        MessageKey::ColClient => "クライアント",
        // 6 cells. `クライアント` is 12 and the narrow Sessions table grants
        // this column about 9, so it rendered as `クライアン` — a truncated
        // word with no ellipsis. The column names the coding tool the session
        // came from, which is what `ツール` says in 6 cells.
        MessageKey::ColClientShort => "ツール",
        MessageKey::ColProvider => "プロバイダ",
        MessageKey::ColSource => "ソース",
        MessageKey::ColMessages => "件数",
        MessageKey::ColMessagesShort => "件数",
        MessageKey::ColInput => "入力",
        MessageKey::ColOutput => "出力",
        MessageKey::ColCacheRead => "C読込",
        MessageKey::ColCacheWrite => "C書込",
        MessageKey::ColSession => "セッション",
        MessageKey::ColProject => "プロジェクト",
        MessageKey::ColDuration => "所要時間",
        MessageKey::ColWorkspace => "ワークスペース",
        MessageKey::ColTotal => "合計",
        MessageKey::ColTurn => "回",
        MessageKey::ColMonth => "月",
        MessageKey::ColHour => "時間",
        MessageKey::ColMinute => "分",
        MessageKey::ColAgent => "エージェント",
        MessageKey::ColSessions => "回数",
        MessageKey::ColSources => "ソース",
        MessageKey::ColModels => "モデル",
        MessageKey::ColRank => "#",
        MessageKey::ColMsPer1k => "ms/1K",
        MessageKey::ColCostPer1M => "コスト/1M",
        MessageKey::ColLastActive => "最終活動",

        MessageKey::OverviewTopModels => "上位モデル",
        MessageKey::OverviewModelsByTokens => "トークン別モデル",
        MessageKey::OverviewModelsByCost => "コスト別モデル",
        MessageKey::OverviewTotal => "合計: ",

        MessageKey::EmptyNoUsageData => {
            "使用量データが見つかりません。'r': 再読み込み, 's': ソース, 'g': グループ化。"
        }
        MessageKey::EmptyNoModelDetailsDay => "この日のモデル詳細がありません。Escで戻る。",

        MessageKey::FooterTokens => " トークン",
        MessageKey::CountModels => "モデル",
        MessageKey::CountAgents => "エージェント",
        MessageKey::CountDay => "日",
        MessageKey::CountDays => "日",
        MessageKey::CountHours => "時間",
        MessageKey::CountMinutes => "分",
        MessageKey::CountMonths => "ヶ月",
        MessageKey::CountSessions => "セッション",
        MessageKey::CountProjects => "プロジェクト",

        MessageKey::SortLabel => "並び替え: ",
        MessageKey::SortDate => "日付",
        MessageKey::SortCost => "コスト",
        MessageKey::SortTokens => "トークン",

        MessageKey::HelpScroll => "↑↓ スクロール • ←→/tab 表示切替",
        MessageKey::HelpSort => "[d/t/c:並替]",
        MessageKey::HelpBack => "[esc:戻る]",
        MessageKey::HelpDetails => "[enter:詳細]",
        MessageKey::HelpToday => "[j:今日]",
        MessageKey::HelpProfile => "[v:プロファイル]",
        MessageKey::HelpSources => "[s:ソース]",
        MessageKey::HelpLanguage => "[k:言語]",
        MessageKey::HelpRefresh => "[r:更新]",
        MessageKey::HelpQuit => "q",

        MessageKey::DialogCloseHint => "Esc 閉じる",
        MessageKey::DialogFilterPlaceholder => "検索語を入力...",
        MessageKey::DialogFilterLabel => "フィルター: ",
        MessageKey::DialogNoResults => "結果がありません",

        MessageKey::LanguageDialogTitle => " 言語を選択 ",
        MessageKey::LanguageDialogHint => "↑↓ 移動 • Enter 選択 • Esc キャンセル",

        MessageKey::ClientDialogTitle => " クライアント ",
        MessageKey::ClientDialogHint => "↑↓ 移動 • Enter 切替 • Esc 閉じる",

        MessageKey::GroupByDialogTitle => " グループ化 ",
        MessageKey::GroupByDialogHint => "↑↓ 移動 • Enter 選択 • Esc 閉じる",
        MessageKey::GroupByModelLabel => "モデル",
        MessageKey::GroupByModelDesc => "モデルごとに1行 (クライアント・プロバイダ統合)",
        MessageKey::GroupByClientModelLabel => "クライアント + モデル",
        MessageKey::GroupByClientModelDesc => "クライアント・モデルのペアごとに1行 (デフォルト)",
        MessageKey::GroupByClientProviderModelLabel => "クライアント + プロバイダ + モデル",
        MessageKey::GroupByClientProviderModelDesc => "最も詳細 — 統合なし",
        MessageKey::GroupByWorkspaceModelLabel => "ワークスペース + モデル",
        MessageKey::GroupByWorkspaceModelDesc => "ワークスペースキー、モデルごとに集計",
        MessageKey::GroupBySessionLabel => "セッション + モデル",
        MessageKey::GroupBySessionDesc => "セッションID・モデルごとに1行 (セッション単位のコスト)",
        MessageKey::GroupByClientSessionLabel => "クライアント + セッション + モデル",
        MessageKey::GroupByClientSessionDesc => "クライアント、セッションID、モデルごとに1行",

        MessageKey::ConfirmYes => "はい",
        MessageKey::ConfirmNo => "いいえ",

        MessageKey::StatusLoadedFromCache => "キャッシュから読み込みました",
        MessageKey::StatusRefreshInProgress => "更新はすでに処理中です",
        MessageKey::StatusLanguageChanged => "言語を変更しました:",

        MessageKey::ColCacheHit => "Hit率",

        MessageKey::TitleDailyUsage => " 日別使用量 ",
        MessageKey::TitleMonthlyUsage => " 月別使用量 ",
        MessageKey::TitleHourlyUsage => " 時間別使用量 ",
        MessageKey::TitleHourlyProfile => " 時間別プロファイル ",
        MessageKey::TitleMinutelyUsage => " 分別使用量 ",
        MessageKey::TitleUsageSummary => " 使用量概要 ",
        MessageKey::TitleAccounts => " アカウント一覧 ",
        MessageKey::TitleSelectedAccount => " 選択されたアカウント ",
        MessageKey::TitleContributionGraph => " コントリビューショングラフ (52週) ",
        MessageKey::TitleDayBreakdown => " 日別内訳 (ESCで閉じる) ",
        MessageKey::TitleDailyDetail => " 日別詳細 ",
        MessageKey::TitleDailyDetailPrefix => " 日別詳細: ",
        MessageKey::TitleDailyBreakdown => " 日別内訳 ",
        MessageKey::TitleDailyBreakdownPrefix => " 日別内訳: ",

        MessageKey::ChartTokensPerDay => "日別トークン",
        MessageKey::ChartTokens => "トークン",

        MessageKey::EmptyNoDailyData => "日別使用量データが見つかりません。'r'キーで更新してください。",
        MessageKey::EmptyNoMonthlyData => "月別使用量データが見つかりません。'r'キーで更新してください。",
        MessageKey::EmptyNoDailyDataMonth => "この月の日別データが見つかりません。Escキーで戻ります。",
        MessageKey::EmptyNoHourlyData => "時間別使用量データが見つかりません。'r'キーで更新してください。",
        MessageKey::EmptyNoMinutelyData => "分単位の使用量データが見つかりません。'r'キーで更新してください。",
        MessageKey::EmptyNoSessionData => "セッション使用量データが見つかりません。'r'キーで更新してください。",
        MessageKey::EmptyNoProjectData => "プロジェクト使用量データが見つかりません。'r'キーで更新してください。",
        MessageKey::EmptyNoDataAvailable => "利用可能なデータがありません",
        MessageKey::EmptyNoDataForDay => "この日のデータはありません",
        MessageKey::EmptyNoBreakdownAvailable => "詳細な内訳はありません",
        MessageKey::EmptyNoAgentCodex => "現在のソースのエージェント内訳は利用できません。\n選択されたソースは通常、通常セッションのエージェントメタデータを記録しません。\n's'を押して別のソースをお試しください。",
        MessageKey::EmptyNoAgentMixed => "現在のソースのエージェント内訳は利用できません。\n一部のソースのみがエージェントメタデータを記録します。\n's'でソースを変更するか、'r'で更新してください。",

        MessageKey::StatusRefreshingBackground => "バックグラウンドでキャッシュデータを更新中...",
        MessageKey::StatusLastUpdated => "最終更新: ",
        MessageKey::StatusLocal => "ローカル",
        MessageKey::StatusDevice => "1台のデバイス",
        MessageKey::StatusAllDevices => " すべてのデバイス: ",

        MessageKey::StatsFavoriteModel => "最も使用したモデル:",
        MessageKey::StatsFavoriteModelShort => "モデル:",
        MessageKey::StatsTotalTokens => "総トークン:",
        MessageKey::StatsTokensShort => "トークン:",
        MessageKey::StatsSessions => "セッション数:",
        MessageKey::StatsTotalCost => "総費用:",
        MessageKey::StatsCostShort => "費用:",
        MessageKey::StatsCurrentStreak => "現在の連続日数:",
        MessageKey::StatsStreakShort => "連続:",
        MessageKey::StatsLongestStreak => "最長連続日数:",
        MessageKey::StatsLongestStreakShort => "最大連続:",
        MessageKey::StatsActiveDays => "アクティブ日数:",
        MessageKey::StatsActiveShort => "アクティブ:",
        MessageKey::StatsLess => "少",
        MessageKey::StatsMore => "多",

        MessageKey::DialogCurrentLabel => "現在: ",
        MessageKey::DialogCancel => "[ キャンセル ]",
        MessageKey::DialogTargetLabel => "対象",
        MessageKey::DialogEffectLabel => "効果",

        MessageKey::PhaseInitializing => "初期化中...",
        MessageKey::PhaseScanningSessions => "セッションデータをスキャン中...",
        MessageKey::PhaseLoadingPricing => "価格データを読み込み中...",
        MessageKey::PhaseFinalizingReport => "レポートの最終処理中...",
        MessageKey::PhaseComplete => "完了",
        MessageKey::PhaseLoadingData => "データを読み込み中...",
        MessageKey::LabelError => "エラー",

        MessageKey::ProfileWhenYouWorkMost => "主な作業時間帯",
        MessageKey::ProfileMostProductiveDay => "最も生産的な曜日",
        MessageKey::ProfilePeakHour => "ピーク時間: ",
        MessageKey::ProfileLegend => "凡例: ",
        MessageKey::ProfileLow => "低",
        MessageKey::ProfileHigh => "高",
        MessageKey::ProfileTotalTokens => "合計トークン",
        MessageKey::ProfileTotalCost => "合計コスト",
        MessageKey::PeriodMorning => "朝",
        MessageKey::PeriodDaytime => "昼",
        MessageKey::PeriodEvening => "夕方",
        MessageKey::PeriodNight => "夜",
        MessageKey::WeekdayMonday => "月曜日",
        MessageKey::WeekdayTuesday => "火曜日",
        MessageKey::WeekdayWednesday => "水曜日",
        MessageKey::WeekdayThursday => "木曜日",
        MessageKey::WeekdayFriday => "金曜日",
        MessageKey::WeekdaySaturday => "土曜日",
        MessageKey::WeekdaySunday => "日曜日",
        MessageKey::ProfileSwitchHint => "[v]: テーブル表示に切り替え",

        MessageKey::StatusLanguageSaveFailed => "言語設定の保存に失敗しました:",

        MessageKey::StatusSyncingUsage => "使用状況を同期中",
        MessageKey::StatusCodexLogin => "Codex ログイン",
        MessageKey::StatusNoData => "データなし",
        MessageKey::StatusNotLoaded => "未ロード",
        MessageKey::StatusSavedSingular => "件保存済み",
        MessageKey::StatusSavedPlural => "件保存済み",
        MessageKey::StatusManagedSingular => "件管理対象",
        MessageKey::StatusManagedPlural => "件管理対象",
        MessageKey::StatusIssueSingular => "件の問題",
        MessageKey::StatusIssuePlural => "件の問題",

        MessageKey::ActionRefresh => "r 更新",
        MessageKey::ActionRefreshSyncing => "r 同期中",
        MessageKey::ActionAddCodex => "a Codex 追加",
        MessageKey::ActionAddingCodex => "a Codex 追加中",
        MessageKey::ActionShowEmails => "m メール表示",
        MessageKey::ActionHideEmails => "m メール非表示",
        MessageKey::ActionReset => "x リセット",

        MessageKey::HeadingAttention => "注意",
        MessageKey::HeadingDiagnostics => "診断",
        MessageKey::HeadingProviders => "プロバイダー",
        MessageKey::HeadingLimits => "制限",
        MessageKey::HeadingActions => "アクション",
        MessageKey::HeadingCreditBank => "クレジットバンク",

        MessageKey::LabelStatus => "状態",
        MessageKey::LabelEmail => "メール",
        MessageKey::LabelCredential => "認証情報",
        MessageKey::LabelCredits => "クレジット",
        // 10 cells; `リセットバンク` is 14 and overflowed the 12-cell gutter.
        MessageKey::LabelResetBank => "リセット枠",

        MessageKey::ColAccount => "アカウント",
        MessageKey::ColPlan => "プラン",
        MessageKey::ColAuth => "認証",
        MessageKey::ColHealth => "ヘルス",
        MessageKey::ColLimit => "制限",
        MessageKey::ColReset => "リセット",
        MessageKey::UsageAccountOrStatus => "アカウント / 状態",

        // Usage: empty and failure states
        MessageKey::UsageEmptyNoData => "使用量データなし",
        MessageKey::UsageEmptyNotLoadedTitle => "サブスクリプションデータ未ロード",
        MessageKey::UsageEmptyNotLoadedHint => "更新でプロバイダーの使用量を同期するか、Codex 追加で別のアカウントを保存してください。",
        MessageKey::UsageFetchFailed => "使用量の取得に失敗しました",
        MessageKey::UsageEmptyNoSubscriptionData => "利用できるサブスクリプションデータがありません",
        MessageKey::UsageNoAttentionNeeded => "注意が必要なアカウントはありません",

        // Usage: overall state vocabulary
        MessageKey::StateSwitchRecommended => "切り替え推奨",
        MessageKey::StateReady => "正常",
        MessageKey::StateReadyWithWarnings => "正常（注意あり）",
        // `残量少`, the same word as `HealthQuotaLow`: both are visible on the
        // same frame and 6 cells fits the State row, so there is no reason for
        // two spellings of one concept.
        MessageKey::StateQuotaLow => "残量少",
        MessageKey::StateUnknown => "不明",

        // Usage: per-account readiness / the Health column
        MessageKey::HealthReady => "正常",
        MessageKey::HealthWatch => "注意",
        MessageKey::HealthQuotaLow => "残量少",
        MessageKey::HealthUnknown => "不明",

        // Usage: summary K/V row labels
        MessageKey::LabelState => "状態",
        MessageKey::LabelActiveAccount => "使用中",
        MessageKey::LabelCapacity => "残量",
        MessageKey::LabelFallback => "代替",
        // `次回リセット` is exactly 12 cells, which left `push_kv_styled`'s
        // 12-cell gutter with zero padding and collided with its value.
        // `リセット日` ("reset date") is 10 and keeps a separating space.
        MessageKey::LabelNextReset => "リセット日",
        MessageKey::LabelAction => "対応",

        // Usage: summary K/V row values
        MessageKey::UsageNoActiveAccount => "使用中のアカウントなし",
        MessageKey::UsageNoReadyFallback => "利用可能な代替なし",
        MessageKey::UsageNoResetData => "リセット情報なし",
        MessageKey::UsagePercentLeft => "% 残り",
        MessageKey::CapacityReady => "正常",
        MessageKey::CapacityWatch => "注意",
        MessageKey::CapacityCritical => "危険",
        MessageKey::CapacityUnknown => "不明",

        // Usage: recommended next action
        MessageKey::ActionChooseActive => "使用するアカウントを選択してください",
        MessageKey::ActionRefreshUnknownLimits => "制限が不明なアカウントを更新してください",
        MessageKey::ActionKeepCurrent => "現在のアカウントを継続",
        MessageKey::ActionMonitorQuota => "使用中の残量を監視してください",
        MessageKey::ActionUsePrefix => "切り替え:",
        MessageKey::ActionWaitForReset => "リセットを待つか更新してください",
        MessageKey::ActionRefreshActive => "使用中のアカウントを更新してください",

        // Usage: account state / the Auth column
        MessageKey::AuthActive => "有効",
        MessageKey::AuthSaved => "保存済",
        MessageKey::AuthManaged => "管理中",
        MessageKey::StateAuthenticated => "認証済み",
        MessageKey::LabelUnknown => "不明",

        // Usage: "+N more ..." counters
        MessageKey::UsageMoreIssues => "件の問題",
        MessageKey::UsageMoreIssuesPlural => "件の問題",
        MessageKey::UsageMoreAtRisk => "件が危険",
        MessageKey::UsageMoreAtRiskPlural => "件が危険",

        // Usage: row-action buttons
        MessageKey::ButtonUseAccount => "このアカウントを使用",
        MessageKey::ButtonRemove => "削除",
        MessageKey::ButtonReset => "リセット",

        // Usage: limits, metrics and the snapshot line
        MessageKey::UsageNoLimits => "制限なし",
        MessageKey::UsageNoQuotaMetrics => "残量指標なし",
        MessageKey::UsageNoQuotaMetricsReturned => "残量指標が返されませんでした",
        MessageKey::UsageSnapshot => "スナップショット",
        MessageKey::UsageAtRisk => "危険",
        MessageKey::UsageEmailsHidden => "メール非表示",

        // Usage: credit-bank expiry rows and counters
        MessageKey::CreditExpiryUnknown => "有効期限不明",
        MessageKey::CreditExpiresPrefix => "有効期限",
        MessageKey::CreditNearestExpires => "最短の有効期限",
        MessageKey::CreditMoreResetCredits => "件のリセット枠",
        MessageKey::CreditMoreResetCreditsPlural => "件のリセット枠",
        // `枠` ("slot/allowance"), not a bare `件`: `3件` is a counter with no
        // noun where English says `3 credits` and this panel's own heading says
        // `リセット枠`. `3枠` carries the noun in the counter, the way Japanese
        // counts allowances.
        MessageKey::CreditCountSingular => "枠",
        MessageKey::CreditCountPlural => "枠",
        MessageKey::CreditAvailableSingular => "件利用可能",
        MessageKey::CreditAvailablePlural => "件利用可能",
        MessageKey::CreditAvailableAcrossAccounts => "件が全アカウントで利用可能",
        MessageKey::UsageNoResetCredits => "リセット枠なし",

        // Usage: the Codex login panel
        MessageKey::CodexLoginTitle => "Codex ログイン",
        MessageKey::CodexLoginImported => "取り込み完了",
        MessageKey::CodexLoginFailed => "失敗",
        MessageKey::CodexLoginRunning => "実行中",
        MessageKey::CodexLoginIdle => "待機",
        MessageKey::CodexLoginCancel => "[中止]",
        MessageKey::CodexLoginDismiss => "[閉じる]",
        MessageKey::CodexLoginWaiting => "codex の出力を待機中...",
        MessageKey::CodexLoginImportedPrefix => "取り込み完了:",

        // Usage: the fetching spinner
        MessageKey::UsageFetchingShort => "使用量を取得中...",
        MessageKey::UsageFetchingLong => "サブスクリプションデータを取得中...",

        // Usage: compact (<48 columns) action-bar labels
        MessageKey::ActionRefreshSyncingShort => "r 同期",
        MessageKey::ActionAddingCodexShort => "a 追加中",
        MessageKey::ActionAddCodexShort => "a 追加",
        MessageKey::ActionShowEmailsShort => "m 表示",
        MessageKey::ActionHideEmailsShort => "m 非表示",

        // Usage: credential provenance wording
        MessageKey::CredentialSavedActive => "保存済み、現在の Codex ログイン",
        MessageKey::CredentialSaved => "保存済み",
        MessageKey::CredentialManagedByPrefix => "管理元:",
        MessageKey::CredentialManagedExternally => "外部で管理",
        MessageKey::UsageCurrentAccount => "使用中のアカウント",
        MessageKey::UsageManagedByPrefix => "管理元:",
        MessageKey::UsageManagedExternally => "外部で管理",


    })
}

const fn tr_zh_cn(key: MessageKey) -> Option<&'static str> {
    Some(match key {
        MessageKey::TabOverview => "概览",
        MessageKey::TabUsage => "使用量",
        MessageKey::TabModels => "模型",
        MessageKey::TabDaily => "每日",
        MessageKey::TabHourly => "每小时",
        MessageKey::TabMinutely => "每分钟",
        MessageKey::TabMonthly => "每月",
        MessageKey::TabSessions => "会话",
        MessageKey::TabProjects => "项目",
        MessageKey::TabStats => "统计",
        MessageKey::TabAgents => "智能体",

        MessageKey::TabOverviewShort => "概览",
        MessageKey::TabUsageShort => "用量",
        MessageKey::TabModelsShort => "模型",
        MessageKey::TabDailyShort => "每日",
        MessageKey::TabHourlyShort => "时别",
        MessageKey::TabMinutelyShort => "分别",
        MessageKey::TabMonthlyShort => "每月",
        MessageKey::TabSessionsShort => "会话",
        MessageKey::TabProjectsShort => "项目",
        MessageKey::TabStatsShort => "统计",
        MessageKey::TabAgentsShort => "智能",

        MessageKey::ColDate => "日期",
        MessageKey::ColCost => "费用",
        MessageKey::ColTokens => "Token",
        MessageKey::ColModel => "模型",
        MessageKey::ColClient => "客户端",
        // 6 cells, same as the full label, which already fits.
        MessageKey::ColClientShort => "客户端",
        MessageKey::ColProvider => "供应商",
        MessageKey::ColSource => "来源",
        MessageKey::ColMessages => "消息",
        MessageKey::ColMessagesShort => "消息",
        MessageKey::ColInput => "输入",
        MessageKey::ColOutput => "输出",
        MessageKey::ColCacheRead => "缓存读取",
        MessageKey::ColCacheWrite => "缓存写入",
        MessageKey::ColSession => "会话",
        MessageKey::ColProject => "项目",
        MessageKey::ColDuration => "用时",
        MessageKey::ColWorkspace => "工作区",
        MessageKey::ColTotal => "总计",
        MessageKey::ColTurn => "轮次",
        MessageKey::ColMonth => "月份",
        MessageKey::ColHour => "小时",
        MessageKey::ColMinute => "分钟",
        MessageKey::ColAgent => "智能体",
        MessageKey::ColSessions => "会话",
        MessageKey::ColSources => "来源",
        MessageKey::ColModels => "模型",
        MessageKey::ColRank => "#",
        MessageKey::ColMsPer1k => "ms/1K",
        MessageKey::ColCostPer1M => "费用/1M",
        MessageKey::ColLastActive => "最近活动",

        MessageKey::OverviewTopModels => "热门模型",
        MessageKey::OverviewModelsByTokens => "按 Token 排序模型",
        MessageKey::OverviewModelsByCost => "按费用排序模型",
        MessageKey::OverviewTotal => "总计: ",

        MessageKey::EmptyNoUsageData => "未找到使用量数据。按 'r' 刷新，'s' 查看来源，'g' 分组。",
        MessageKey::EmptyNoModelDetailsDay => "当天未找到模型详细信息。按 Esc 返回。",

        MessageKey::FooterTokens => " Token",
        MessageKey::CountModels => "个模型",
        MessageKey::CountAgents => "个智能体",
        MessageKey::CountDay => "天",
        MessageKey::CountDays => "天",
        MessageKey::CountHours => "小时",
        MessageKey::CountMinutes => "分钟",
        MessageKey::CountMonths => "个月",
        MessageKey::CountSessions => "个会话",
        MessageKey::CountProjects => "个项目",

        MessageKey::SortLabel => "排序: ",
        MessageKey::SortDate => "日期",
        MessageKey::SortCost => "费用",
        MessageKey::SortTokens => "Token",

        MessageKey::HelpScroll => "↑↓ 滚动 • ←→/tab 切换视图",
        MessageKey::HelpSort => "[d/t/c:排序]",
        MessageKey::HelpBack => "[esc:返回]",
        MessageKey::HelpDetails => "[enter:详情]",
        MessageKey::HelpToday => "[j:今日]",
        MessageKey::HelpProfile => "[v:分析图]",
        MessageKey::HelpSources => "[s:来源]",
        MessageKey::HelpLanguage => "[k:语言]",
        MessageKey::HelpRefresh => "[r:刷新]",
        MessageKey::HelpQuit => "q",

        MessageKey::DialogCloseHint => "Esc 关闭",
        MessageKey::DialogFilterPlaceholder => "输入以过滤...",
        MessageKey::DialogFilterLabel => "过滤: ",
        MessageKey::DialogNoResults => "无匹配结果",

        MessageKey::LanguageDialogTitle => " 选择语言 ",
        MessageKey::LanguageDialogHint => "↑↓ 移动 • Enter 选择 • Esc 取消",

        MessageKey::ClientDialogTitle => " 客户端 ",
        MessageKey::ClientDialogHint => "↑↓ 移动 • Enter 切换 • Esc 关闭",

        MessageKey::GroupByDialogTitle => " 分组依据 ",
        MessageKey::GroupByDialogHint => "↑↓ 移动 • Enter 选择 • Esc 关闭",
        MessageKey::GroupByModelLabel => "模型",
        MessageKey::GroupByModelDesc => "每个模型一行 (合并客户端和供应商)",
        MessageKey::GroupByClientModelLabel => "客户端 + 模型",
        MessageKey::GroupByClientModelDesc => "每个客户端与模型组合一行 (默认)",
        MessageKey::GroupByClientProviderModelLabel => "客户端 + 供应商 + 模型",
        MessageKey::GroupByClientProviderModelDesc => "最详细 — 不合并",
        MessageKey::GroupByWorkspaceModelLabel => "工作区 + 模型",
        MessageKey::GroupByWorkspaceModelDesc => "按工作区及模型归类本地用量",
        MessageKey::GroupBySessionLabel => "会话 + 模型",
        MessageKey::GroupBySessionDesc => "每个会话ID及模型一行 (按会话核算费用)",
        MessageKey::GroupByClientSessionLabel => "客户端 + 会话 + 模型",
        MessageKey::GroupByClientSessionDesc => "每个客户端、会话ID及模型一行",

        MessageKey::ConfirmYes => "是",
        MessageKey::ConfirmNo => "否",

        MessageKey::StatusLoadedFromCache => "已从缓存载入",
        MessageKey::StatusRefreshInProgress => "刷新已在进行中",
        MessageKey::StatusLanguageChanged => "语言已更改为",

        MessageKey::ColCacheHit => "缓存✕",

        MessageKey::TitleDailyUsage => " 每日用量 ",
        MessageKey::TitleMonthlyUsage => " 每月用量 ",
        MessageKey::TitleHourlyUsage => " 每小时用量 ",
        MessageKey::TitleHourlyProfile => " 每小时概况 ",
        MessageKey::TitleMinutelyUsage => " 每分钟用量 ",
        MessageKey::TitleUsageSummary => " 用量摘要 ",
        MessageKey::TitleAccounts => " 账户列表 ",
        MessageKey::TitleSelectedAccount => " 所选账户 ",
        MessageKey::TitleContributionGraph => " 贡献图 (52周) ",
        MessageKey::TitleDayBreakdown => " 每日明细 (ESC关闭) ",
        MessageKey::TitleDailyDetail => " 每日详情 ",
        MessageKey::TitleDailyDetailPrefix => " 每日详情: ",
        MessageKey::TitleDailyBreakdown => " 每日明细 ",
        MessageKey::TitleDailyBreakdownPrefix => " 每日明细: ",

        MessageKey::ChartTokensPerDay => "每日 Token",
        MessageKey::ChartTokens => "Token",

        MessageKey::EmptyNoDailyData => "未找到每日用量数据。按 'r' 刷新。",
        MessageKey::EmptyNoMonthlyData => "未找到每月用量数据。按 'r' 刷新。",
        MessageKey::EmptyNoDailyDataMonth => "未找到该月的每日数据。按 Esc 返回。",
        MessageKey::EmptyNoHourlyData => "未找到每小时用量数据。按 'r' 刷新。",
        MessageKey::EmptyNoMinutelyData => "未找到每分钟用量数据。按 'r' 刷新。",
        MessageKey::EmptyNoSessionData => "未找到会话用量数据。按 'r' 刷新。",
        MessageKey::EmptyNoProjectData => "未找到项目用量数据。按 'r' 刷新。",
        MessageKey::EmptyNoDataAvailable => "无可用数据",
        MessageKey::EmptyNoDataForDay => "该日无数据",
        MessageKey::EmptyNoBreakdownAvailable => "无可用详细明细",
        MessageKey::EmptyNoAgentCodex => "当前数据源无可用智能体明细。\n所选源通常不记录常规会话的智能体元数据。\n按 's' 尝试其他源。",
        MessageKey::EmptyNoAgentMixed => "当前数据源无可用智能体明细。\n仅部分源记录智能体元数据。\n按 's' 更改数据源或按 'r' 刷新。",

        MessageKey::StatusRefreshingBackground => "正在后台刷新缓存数据...",
        MessageKey::StatusLastUpdated => "最后更新: ",
        MessageKey::StatusLocal => "本地",
        MessageKey::StatusDevice => "1台设备",
        MessageKey::StatusAllDevices => " 所有设备: ",

        MessageKey::StatsFavoriteModel => "常用模型:",
        MessageKey::StatsFavoriteModelShort => "模型:",
        MessageKey::StatsTotalTokens => "Token 总数:",
        MessageKey::StatsTokensShort => "Token:",
        MessageKey::StatsSessions => "会话数:",
        MessageKey::StatsTotalCost => "总费用:",
        MessageKey::StatsCostShort => "费用:",
        MessageKey::StatsCurrentStreak => "当前连续天数:",
        MessageKey::StatsStreakShort => "连续:",
        MessageKey::StatsLongestStreak => "最长连续天数:",
        MessageKey::StatsLongestStreakShort => "最大连续:",
        MessageKey::StatsActiveDays => "活跃天数:",
        MessageKey::StatsActiveShort => "活跃:",
        MessageKey::StatsLess => "少",
        MessageKey::StatsMore => "多",

        MessageKey::DialogCurrentLabel => "当前: ",
        MessageKey::DialogCancel => "[ 取消 ]",
        MessageKey::DialogTargetLabel => "目标",
        MessageKey::DialogEffectLabel => "效果",

        MessageKey::PhaseInitializing => "正在初始化...",
        MessageKey::PhaseScanningSessions => "正在扫描会话数据...",
        MessageKey::PhaseLoadingPricing => "正在加载定价数据...",
        MessageKey::PhaseFinalizingReport => "正在生成报告...",
        MessageKey::PhaseComplete => "完成",
        MessageKey::PhaseLoadingData => "正在加载数据...",
        MessageKey::LabelError => "错误",

        MessageKey::ProfileWhenYouWorkMost => "主要工作时段",
        MessageKey::ProfileMostProductiveDay => "最高效的工作日",
        MessageKey::ProfilePeakHour => "高峰时段: ",
        MessageKey::ProfileLegend => "图例: ",
        MessageKey::ProfileLow => "低",
        MessageKey::ProfileHigh => "高",
        MessageKey::ProfileTotalTokens => "Token 总数",
        MessageKey::ProfileTotalCost => "总费用",
        MessageKey::PeriodMorning => "早晨",
        MessageKey::PeriodDaytime => "白天",
        MessageKey::PeriodEvening => "傍晚",
        MessageKey::PeriodNight => "夜间",
        MessageKey::WeekdayMonday => "周一",
        MessageKey::WeekdayTuesday => "周二",
        MessageKey::WeekdayWednesday => "周三",
        MessageKey::WeekdayThursday => "周四",
        MessageKey::WeekdayFriday => "周五",
        MessageKey::WeekdaySaturday => "周六",
        MessageKey::WeekdaySunday => "周日",
        MessageKey::ProfileSwitchHint => "[v]: 切换到表格视图",

        MessageKey::StatusLanguageSaveFailed => "保存语言设置失败:",

        MessageKey::StatusSyncingUsage => "正在同步用量",
        MessageKey::StatusCodexLogin => "Codex 登录",
        MessageKey::StatusNoData => "无数据",
        MessageKey::StatusNotLoaded => "未加载",
        MessageKey::StatusSavedSingular => "个已保存",
        MessageKey::StatusSavedPlural => "个已保存",
        MessageKey::StatusManagedSingular => "个托管",
        MessageKey::StatusManagedPlural => "个托管",
        MessageKey::StatusIssueSingular => "个问题",
        MessageKey::StatusIssuePlural => "个问题",

        MessageKey::ActionRefresh => "r 刷新",
        MessageKey::ActionRefreshSyncing => "r 同步中",
        MessageKey::ActionAddCodex => "a 添加 Codex",
        MessageKey::ActionAddingCodex => "a 正在添加 Codex",
        MessageKey::ActionShowEmails => "m 显示邮箱",
        MessageKey::ActionHideEmails => "m 隐藏邮箱",
        MessageKey::ActionReset => "x 重置",

        MessageKey::HeadingAttention => "注意",
        MessageKey::HeadingDiagnostics => "诊断",
        MessageKey::HeadingProviders => "提供商",
        MessageKey::HeadingLimits => "限制",
        MessageKey::HeadingActions => "操作",
        MessageKey::HeadingCreditBank => "积分额度",

        MessageKey::LabelStatus => "状态",
        MessageKey::LabelEmail => "邮箱",
        MessageKey::LabelCredential => "凭据",
        MessageKey::LabelCredits => "积分",
        MessageKey::LabelResetBank => "重置额度",

        MessageKey::ColAccount => "账户",
        MessageKey::ColPlan => "套餐",
        MessageKey::ColAuth => "认证",
        MessageKey::ColHealth => "健康度",
        MessageKey::ColLimit => "限制",
        MessageKey::ColReset => "重置",
        MessageKey::UsageAccountOrStatus => "账户 / 状态",

        // Usage: empty and failure states
        MessageKey::UsageEmptyNoData => "无用量数据",
        MessageKey::UsageEmptyNotLoadedTitle => "尚未加载订阅数据",
        MessageKey::UsageEmptyNotLoadedHint => "用“刷新”同步提供商用量，或用“添加 Codex”保存其他账户。",
        MessageKey::UsageFetchFailed => "用量获取失败",
        MessageKey::UsageEmptyNoSubscriptionData => "没有可用的订阅数据",
        MessageKey::UsageNoAttentionNeeded => "没有需要注意的账户",

        // Usage: overall state vocabulary
        MessageKey::StateSwitchRecommended => "建议切换账户",
        MessageKey::StateReady => "正常",
        MessageKey::StateReadyWithWarnings => "正常（有警告）",
        MessageKey::StateQuotaLow => "配额不足",
        MessageKey::StateUnknown => "未知",

        // Usage: per-account readiness / the Health column
        MessageKey::HealthReady => "正常",
        MessageKey::HealthWatch => "注意",
        MessageKey::HealthQuotaLow => "配额不足",
        MessageKey::HealthUnknown => "未知",

        // Usage: summary K/V row labels
        MessageKey::LabelState => "状态",
        MessageKey::LabelActiveAccount => "当前账户",
        MessageKey::LabelCapacity => "余量",
        MessageKey::LabelFallback => "备用",
        MessageKey::LabelNextReset => "下次重置",
        MessageKey::LabelAction => "操作",

        // Usage: summary K/V row values
        MessageKey::UsageNoActiveAccount => "没有当前账户",
        MessageKey::UsageNoReadyFallback => "没有可用备用账户",
        MessageKey::UsageNoResetData => "没有重置信息",
        MessageKey::UsagePercentLeft => "% 剩余",
        MessageKey::CapacityReady => "正常",
        MessageKey::CapacityWatch => "注意",
        MessageKey::CapacityCritical => "紧急",
        MessageKey::CapacityUnknown => "未知",

        // Usage: recommended next action
        MessageKey::ActionChooseActive => "请选择一个当前账户",
        MessageKey::ActionRefreshUnknownLimits => "刷新限制未知的账户",
        MessageKey::ActionKeepCurrent => "保持当前账户",
        MessageKey::ActionMonitorQuota => "关注当前账户配额",
        MessageKey::ActionUsePrefix => "切换到",
        MessageKey::ActionWaitForReset => "等待重置或刷新",
        MessageKey::ActionRefreshActive => "刷新当前账户",

        // Usage: account state / the Auth column
        MessageKey::AuthActive => "活跃",
        MessageKey::AuthSaved => "已保存",
        MessageKey::AuthManaged => "托管",
        MessageKey::StateAuthenticated => "已认证",
        MessageKey::LabelUnknown => "未知",

        // Usage: "+N more ..." counters
        MessageKey::UsageMoreIssues => "个问题",
        MessageKey::UsageMoreIssuesPlural => "个问题",
        MessageKey::UsageMoreAtRisk => "个有风险",
        MessageKey::UsageMoreAtRiskPlural => "个有风险",

        // Usage: row-action buttons
        MessageKey::ButtonUseAccount => "使用此账户",
        MessageKey::ButtonRemove => "移除",
        MessageKey::ButtonReset => "重置",

        // Usage: limits, metrics and the snapshot line
        MessageKey::UsageNoLimits => "无限制信息",
        MessageKey::UsageNoQuotaMetrics => "无配额指标",
        MessageKey::UsageNoQuotaMetricsReturned => "未返回配额指标",
        MessageKey::UsageSnapshot => "概览",
        MessageKey::UsageAtRisk => "有风险",
        MessageKey::UsageEmailsHidden => "已隐藏邮箱",

        // Usage: credit-bank expiry rows and counters
        MessageKey::CreditExpiryUnknown => "有效期未知",
        MessageKey::CreditExpiresPrefix => "到期",
        MessageKey::CreditNearestExpires => "最近到期",
        MessageKey::CreditMoreResetCredits => "个重置额度",
        MessageKey::CreditMoreResetCreditsPlural => "个重置额度",
        MessageKey::CreditCountSingular => "个额度",
        MessageKey::CreditCountPlural => "个额度",
        MessageKey::CreditAvailableSingular => "个可用",
        MessageKey::CreditAvailablePlural => "个可用",
        MessageKey::CreditAvailableAcrossAccounts => "个可用（所有账户）",
        MessageKey::UsageNoResetCredits => "无重置额度",

        // Usage: the Codex login panel
        MessageKey::CodexLoginTitle => "Codex 登录",
        MessageKey::CodexLoginImported => "已导入",
        MessageKey::CodexLoginFailed => "失败",
        MessageKey::CodexLoginRunning => "进行中",
        MessageKey::CodexLoginIdle => "空闲",
        MessageKey::CodexLoginCancel => "[取消]",
        MessageKey::CodexLoginDismiss => "[关闭]",
        MessageKey::CodexLoginWaiting => "正在等待 codex 输出...",
        MessageKey::CodexLoginImportedPrefix => "已导入:",

        // Usage: the fetching spinner
        MessageKey::UsageFetchingShort => "正在获取用量...",
        MessageKey::UsageFetchingLong => "正在获取订阅数据...",

        // Usage: compact (<48 columns) action-bar labels
        MessageKey::ActionRefreshSyncingShort => "r 同步",
        MessageKey::ActionAddingCodexShort => "a 添加中",
        MessageKey::ActionAddCodexShort => "a 添加",
        MessageKey::ActionShowEmailsShort => "m 显示",
        MessageKey::ActionHideEmailsShort => "m 隐藏",

        // Usage: credential provenance wording
        MessageKey::CredentialSavedActive => "已保存，当前 Codex 登录",
        MessageKey::CredentialSaved => "已保存",
        MessageKey::CredentialManagedByPrefix => "托管方:",
        MessageKey::CredentialManagedExternally => "外部托管",
        MessageKey::UsageCurrentAccount => "当前账户",
        MessageKey::UsageManagedByPrefix => "托管方:",
        MessageKey::UsageManagedExternally => "外部托管",


    })
}

const fn tr_fr(key: MessageKey) -> Option<&'static str> {
    Some(match key {
        MessageKey::TabOverview => "Aperçu",
        MessageKey::TabUsage => "Utilisation",
        MessageKey::TabModels => "Modèles",
        MessageKey::TabDaily => "Quotidien",
        MessageKey::TabHourly => "Horaire",
        MessageKey::TabMinutely => "Par minute",
        MessageKey::TabMonthly => "Mensuel",
        MessageKey::TabSessions => "Sessions",
        MessageKey::TabProjects => "Projets",
        MessageKey::TabStats => "Statistiques",
        MessageKey::TabAgents => "Agents",

        MessageKey::TabOverviewShort => "Ape",
        MessageKey::TabUsageShort => "Uti",
        MessageKey::TabModelsShort => "Mod",
        MessageKey::TabDailyShort => "Jor",
        MessageKey::TabHourlyShort => "Hor",
        MessageKey::TabMinutelyShort => "Min",
        MessageKey::TabMonthlyShort => "Moi",
        MessageKey::TabSessionsShort => "Ses",
        MessageKey::TabProjectsShort => "Prj",
        MessageKey::TabStatsShort => "Sta",
        MessageKey::TabAgentsShort => "Agt",

        MessageKey::ColDate => "Date",
        MessageKey::ColCost => "Coût",
        MessageKey::ColTokens => "Jetons",
        MessageKey::ColModel => "Modèle",
        MessageKey::ColClient => "Client",
        MessageKey::ColClientShort => "Client",
        MessageKey::ColProvider => "Fournisseur",
        MessageKey::ColSource => "Source",
        MessageKey::ColMessages => "Msgs",
        MessageKey::ColMessagesShort => "Msgs",
        MessageKey::ColInput => "Entrée",
        MessageKey::ColOutput => "Sortie",
        MessageKey::ColCacheRead => "Cache L",
        MessageKey::ColCacheWrite => "Cache É",
        MessageKey::ColSession => "Session",
        MessageKey::ColProject => "Projet",
        MessageKey::ColDuration => "Durée",
        MessageKey::ColWorkspace => "Espace",
        MessageKey::ColTotal => "Total",
        MessageKey::ColTurn => "Tour",
        MessageKey::ColMonth => "Mois",
        MessageKey::ColHour => "Heure",
        MessageKey::ColMinute => "Minute",
        MessageKey::ColAgent => "Agent",
        MessageKey::ColSessions => "Sessions",
        MessageKey::ColSources => "Sources",
        MessageKey::ColModels => "Modèles",
        MessageKey::ColRank => "#",
        MessageKey::ColMsPer1k => "ms/1K",
        MessageKey::ColCostPer1M => "Coût/1M",
        MessageKey::ColLastActive => "Dernière act.",

        MessageKey::OverviewTopModels => "Top modèles",
        MessageKey::OverviewModelsByTokens => "Modèles par jetons",
        MessageKey::OverviewModelsByCost => "Modèles par coût",
        MessageKey::OverviewTotal => "Total: ",

        MessageKey::EmptyNoUsageData => {
            "Aucune donnée d'utilisation. Appuyez sur 'r' pour rafraîchir, 's' pour sources, 'g' pour grouper."
        }
        MessageKey::EmptyNoModelDetailsDay => {
            "Aucun détail de modèle pour ce jour. Appuyez sur Échap pour revenir."
        }

        MessageKey::FooterTokens => " jetons",
        MessageKey::CountModels => "modèles",
        MessageKey::CountAgents => "agents",
        MessageKey::CountDay => "jour",
        MessageKey::CountDays => "jours",
        MessageKey::CountHours => "heures",
        MessageKey::CountMinutes => "minutes",
        MessageKey::CountMonths => "mois",
        MessageKey::CountSessions => "sessions",
        MessageKey::CountProjects => "projets",

        MessageKey::SortLabel => "Tri: ",
        MessageKey::SortDate => "Date",
        MessageKey::SortCost => "Coût",
        MessageKey::SortTokens => "Jetons",

        MessageKey::HelpScroll => "↑↓ défiler • ←→/tab vue",
        MessageKey::HelpSort => "[d/t/c:tri]",
        MessageKey::HelpBack => "[esc:retour]",
        MessageKey::HelpDetails => "[enter:détails]",
        MessageKey::HelpToday => "[j:aujourd'hui]",
        MessageKey::HelpProfile => "[v:profil]",
        MessageKey::HelpSources => "[s:sources]",
        MessageKey::HelpLanguage => "[k:langue]",
        MessageKey::HelpRefresh => "[r:rafraîchir]",
        MessageKey::HelpQuit => "q",

        MessageKey::DialogCloseHint => "Esc fermer",
        MessageKey::DialogFilterPlaceholder => "Filtrer...",
        MessageKey::DialogFilterLabel => "Filtre: ",
        MessageKey::DialogNoResults => "Aucun résultat",

        MessageKey::LanguageDialogTitle => " Choisir la langue ",
        MessageKey::LanguageDialogHint => "↑↓ naviguer • Enter sélectionner • Esc annuler",

        MessageKey::ClientDialogTitle => " Clients ",
        MessageKey::ClientDialogHint => "↑↓ naviguer • Enter basculer • Esc fermer",

        MessageKey::GroupByDialogTitle => " Grouper par ",
        MessageKey::GroupByDialogHint => "↑↓ naviguer • Enter sélectionner • Esc fermer",
        MessageKey::GroupByModelLabel => "Modèle",
        MessageKey::GroupByModelDesc => "Une ligne par modèle (fusionner clients et fournisseurs)",
        MessageKey::GroupByClientModelLabel => "Client + Modèle",
        MessageKey::GroupByClientModelDesc => "Une ligne par paire client-modèle (par défaut)",
        MessageKey::GroupByClientProviderModelLabel => "Client + Fournisseur + Modèle",
        MessageKey::GroupByClientProviderModelDesc => "Le plus granulaire — sans fusion",
        MessageKey::GroupByWorkspaceModelLabel => "Espace de travail + Modèle",
        MessageKey::GroupByWorkspaceModelDesc => "Grouper par clé d'espace, puis par modèle",
        MessageKey::GroupBySessionLabel => "Session + Modèle",
        MessageKey::GroupBySessionDesc => "Une ligne par ID de session et modèle",
        MessageKey::GroupByClientSessionLabel => "Client + Session + Modèle",
        MessageKey::GroupByClientSessionDesc => "Une ligne par client, ID de session et modèle",

        MessageKey::ConfirmYes => "Oui",
        MessageKey::ConfirmNo => "Non",

        MessageKey::StatusLoadedFromCache => "Chargé depuis le cache",
        MessageKey::StatusRefreshInProgress => "Actualisation déjà en cours",
        MessageKey::StatusLanguageChanged => "Langue changée en",

        MessageKey::ColCacheHit => "Cache✕",

        MessageKey::TitleDailyUsage => " Utilisation quotidienne ",
        MessageKey::TitleMonthlyUsage => " Utilisation mensuelle ",
        MessageKey::TitleHourlyUsage => " Utilisation horaire ",
        MessageKey::TitleHourlyProfile => " Profil horaire ",
        MessageKey::TitleMinutelyUsage => " Utilisation par minute ",
        MessageKey::TitleUsageSummary => " Résumé de l'utilisation ",
        MessageKey::TitleAccounts => " Comptes ",
        MessageKey::TitleSelectedAccount => " Compte sélectionné ",
        MessageKey::TitleContributionGraph => " Graphique de contribution (52 semaines) ",
        MessageKey::TitleDayBreakdown => " Détail du jour (Échap pour fermer) ",
        MessageKey::TitleDailyDetail => " Détail quotidien ",
        MessageKey::TitleDailyDetailPrefix => " Détail quotidien : ",
        MessageKey::TitleDailyBreakdown => " Détail quotidien ",
        MessageKey::TitleDailyBreakdownPrefix => " Détail quotidien : ",

        MessageKey::ChartTokensPerDay => "Tokens par jour",
        MessageKey::ChartTokens => "Tokens",

        MessageKey::EmptyNoDailyData => "Aucune donnée d'utilisation quotidienne trouvée. Appuyez sur 'r' pour actualiser.",
        MessageKey::EmptyNoMonthlyData => "Aucune donnée d'utilisation mensuelle trouvée. Appuyez sur 'r' pour actualiser.",
        MessageKey::EmptyNoDailyDataMonth => "Aucune donnée quotidienne trouvée pour ce mois. Appuyez sur Échap pour revenir.",
        MessageKey::EmptyNoHourlyData => "Aucune donnée d'utilisation horaire trouvée. Appuyez sur 'r' pour actualiser.",
        MessageKey::EmptyNoMinutelyData => "Aucune donnée minute trouvée. Appuyez sur 'r' pour actualiser.",
        MessageKey::EmptyNoSessionData => "Aucune donnée d'utilisation de session trouvée. Appuyez sur 'r' pour actualiser.",
        MessageKey::EmptyNoProjectData => "Aucune donnée d'utilisation de projet trouvée. Appuyez sur 'r' pour actualiser.",
        MessageKey::EmptyNoDataAvailable => "Aucune donnée disponible",
        MessageKey::EmptyNoDataForDay => "Aucune donnée pour ce jour",
        MessageKey::EmptyNoBreakdownAvailable => "Aucun détail disponible",
        MessageKey::EmptyNoAgentCodex => "Aucune ventilation par agent n'est disponible pour les sources actuelles.\nLa source sélectionnée n'enregistre généralement pas de métadonnées d'agent pour les sessions régulières.\nAppuyez sur 's' pour essayer une autre source.",
        MessageKey::EmptyNoAgentMixed => "Aucune ventilation par agent n'est disponible pour les sources actuelles.\nSeules certaines sources enregistrent des métadonnées d'agent.\nAppuyez sur 's' pour changer de source ou 'r' pour actualiser.",

        MessageKey::StatusRefreshingBackground => "Actualisation des données en cache en arrière-plan...",
        MessageKey::StatusLastUpdated => "Dernière mise à jour : ",
        MessageKey::StatusLocal => "local",
        MessageKey::StatusDevice => "1 appareil",
        MessageKey::StatusAllDevices => " tous les appareils : ",

        MessageKey::StatsFavoriteModel => "Modèle préféré :",
        MessageKey::StatsFavoriteModelShort => "Modèle :",
        MessageKey::StatsTotalTokens => "Total des tokens :",
        MessageKey::StatsTokensShort => "Tokens :",
        MessageKey::StatsSessions => "Sessions :",
        MessageKey::StatsTotalCost => "Coût total :",
        MessageKey::StatsCostShort => "Coût :",
        MessageKey::StatsCurrentStreak => "Série actuelle :",
        MessageKey::StatsStreakShort => "Série :",
        MessageKey::StatsLongestStreak => "Plus longue série :",
        MessageKey::StatsLongestStreakShort => "Série max :",
        MessageKey::StatsActiveDays => "Jours actifs :",
        MessageKey::StatsActiveShort => "Actif :",
        MessageKey::StatsLess => "Moins",
        MessageKey::StatsMore => "Plus",

        MessageKey::DialogCurrentLabel => "Actuel : ",
        MessageKey::DialogCancel => "[ Annuler ]",
        MessageKey::DialogTargetLabel => "Cible",
        MessageKey::DialogEffectLabel => "Effet",

        MessageKey::PhaseInitializing => "Initialisation...",
        MessageKey::PhaseScanningSessions => "Analyse des données de session...",
        MessageKey::PhaseLoadingPricing => "Chargement des tarifs...",
        MessageKey::PhaseFinalizingReport => "Finalisation du rapport...",
        MessageKey::PhaseComplete => "Terminé",
        MessageKey::PhaseLoadingData => "Chargement des données...",
        MessageKey::LabelError => "Erreur",

        MessageKey::ProfileWhenYouWorkMost => "Période d'activité principale",
        MessageKey::ProfileMostProductiveDay => "Jour le plus productif",
        MessageKey::ProfilePeakHour => "Heure de pointe : ",
        MessageKey::ProfileLegend => "Légende : ",
        MessageKey::ProfileLow => "bas",
        MessageKey::ProfileHigh => "haut",
        MessageKey::ProfileTotalTokens => "tokens totaux",
        MessageKey::ProfileTotalCost => "coût total",
        MessageKey::PeriodMorning => "Matin",
        MessageKey::PeriodDaytime => "Journée",
        MessageKey::PeriodEvening => "Soirée",
        MessageKey::PeriodNight => "Nuit",
        MessageKey::WeekdayMonday => "Lundi",
        MessageKey::WeekdayTuesday => "Mardi",
        MessageKey::WeekdayWednesday => "Mercredi",
        MessageKey::WeekdayThursday => "Jeudi",
        MessageKey::WeekdayFriday => "Vendredi",
        MessageKey::WeekdaySaturday => "Samedi",
        MessageKey::WeekdaySunday => "Dimanche",
        MessageKey::ProfileSwitchHint => "[v] : basculer vers la vue tableau",

        MessageKey::StatusLanguageSaveFailed => "Échec de l'enregistrement de la langue :",

        MessageKey::StatusSyncingUsage => "Synchronisation de l'utilisation",
        MessageKey::StatusCodexLogin => "Connexion Codex",
        MessageKey::StatusNoData => "Aucune donnée",
        MessageKey::StatusNotLoaded => "Non chargé",
        MessageKey::StatusSavedSingular => "enregistré",
        MessageKey::StatusSavedPlural => "enregistrés",
        MessageKey::StatusManagedSingular => "géré",
        MessageKey::StatusManagedPlural => "gérés",
        MessageKey::StatusIssueSingular => "problème",
        MessageKey::StatusIssuePlural => "problèmes",

        MessageKey::ActionRefresh => "r Actualiser",
        MessageKey::ActionRefreshSyncing => "r Synchronisation",
        MessageKey::ActionAddCodex => "a Ajouter Codex",
        MessageKey::ActionAddingCodex => "a Ajout de Codex",
        MessageKey::ActionShowEmails => "m Afficher e-mails",
        MessageKey::ActionHideEmails => "m Masquer e-mails",
        MessageKey::ActionReset => "x Réinitialiser",

        MessageKey::HeadingAttention => "Attention",
        MessageKey::HeadingDiagnostics => "Diagnostics",
        MessageKey::HeadingProviders => "Fournisseurs",
        MessageKey::HeadingLimits => "Limites",
        MessageKey::HeadingActions => "Actions",
        MessageKey::HeadingCreditBank => "Banque de crédits",

        MessageKey::LabelStatus => "Statut",
        MessageKey::LabelEmail => "E-mail",
        MessageKey::LabelCredential => "Identifiant",
        MessageKey::LabelCredits => "Crédits",
        // `Crédits`, the heading's own noun: `HeadingCreditBank` is `Banque de
        // crédits` and both are on screen together, so `Réserve` read as a
        // second name for one pool. 7 cells, inside the 11-cell gutter.
        MessageKey::LabelResetBank => "Crédits",

        MessageKey::ColAccount => "Compte",
        MessageKey::ColPlan => "Forfait",
        MessageKey::ColAuth => "Auth",
        MessageKey::ColHealth => "Santé",
        MessageKey::ColLimit => "Limite",
        MessageKey::ColReset => "Réinit.",
        MessageKey::UsageAccountOrStatus => "Compte / Statut",

        // Usage: empty and failure states
        MessageKey::UsageEmptyNoData => "Aucune donnée",
        MessageKey::UsageEmptyNotLoadedTitle => "Aucune donnée d'abonnement chargée",
        MessageKey::UsageEmptyNotLoadedHint => "Utilisez Actualiser pour synchroniser l'utilisation, ou Ajouter Codex pour enregistrer un autre compte.",
        MessageKey::UsageFetchFailed => "Échec de récupération de l'utilisation",
        MessageKey::UsageEmptyNoSubscriptionData => "Aucune donnée d'abonnement disponible",
        MessageKey::UsageNoAttentionNeeded => "Aucun compte ne nécessite d'attention",

        // Usage: overall state vocabulary
        MessageKey::StateSwitchRecommended => "Changement conseillé",
        MessageKey::StateReady => "Prêt",
        MessageKey::StateReadyWithWarnings => "Prêt avec avertissements",
        MessageKey::StateQuotaLow => "Quota faible",
        MessageKey::StateUnknown => "Inconnu",

        // Usage: per-account readiness / the Health column
        MessageKey::HealthReady => "Prêt",
        MessageKey::HealthWatch => "À suivre",
        MessageKey::HealthQuotaLow => "Faible",
        MessageKey::HealthUnknown => "Inconnu",

        // Usage: summary K/V row labels
        MessageKey::LabelState => "Statut",
        // `Compte actif` is exactly 12 cells, and `push_kv_styled` pads its key
        // to 12: the padding came out empty and the value was jammed against
        // the label with no separating space. 5 cells leaves the gutter intact.
        MessageKey::LabelActiveAccount => "Actif",
        MessageKey::LabelCapacity => "Capacité",
        MessageKey::LabelFallback => "Secours",
        MessageKey::LabelNextReset => "Réinit.",
        MessageKey::LabelAction => "Action",

        // Usage: summary K/V row values
        MessageKey::UsageNoActiveAccount => "Aucun compte actif",
        MessageKey::UsageNoReadyFallback => "Aucun secours disponible",
        MessageKey::UsageNoResetData => "Aucune donnée de réinit.",
        MessageKey::UsagePercentLeft => "% restants",
        MessageKey::CapacityReady => "prêt",
        MessageKey::CapacityWatch => "à suivre",
        MessageKey::CapacityCritical => "critique",
        MessageKey::CapacityUnknown => "inconnu",

        // Usage: recommended next action
        MessageKey::ActionChooseActive => "Choisissez un compte actif",
        MessageKey::ActionRefreshUnknownLimits => "Actualisez les comptes aux limites inconnues",
        MessageKey::ActionKeepCurrent => "Conserver le compte actuel",
        MessageKey::ActionMonitorQuota => "Surveillez le quota actif",
        MessageKey::ActionUsePrefix => "Basculer vers",
        MessageKey::ActionWaitForReset => "Attendez la réinit. ou actualisez",
        MessageKey::ActionRefreshActive => "Actualisez le compte actif",

        // Usage: account state / the Auth column
        MessageKey::AuthActive => "Actif",
        MessageKey::AuthSaved => "Enreg.",
        MessageKey::AuthManaged => "Géré",
        MessageKey::StateAuthenticated => "Authentifié",
        MessageKey::LabelUnknown => "Inconnu",

        // Usage: "+N more ..." counters
        MessageKey::UsageMoreIssues => "autre problème",
        MessageKey::UsageMoreIssuesPlural => "autres problèmes",
        MessageKey::UsageMoreAtRisk => "autre à risque",
        MessageKey::UsageMoreAtRiskPlural => "autres à risque",

        // Usage: row-action buttons
        MessageKey::ButtonUseAccount => "Utiliser ce compte",
        MessageKey::ButtonRemove => "Supprimer",
        MessageKey::ButtonReset => "Réinitialiser",

        // Usage: limits, metrics and the snapshot line
        MessageKey::UsageNoLimits => "Aucune limite",
        MessageKey::UsageNoQuotaMetrics => "Aucun indicateur",
        MessageKey::UsageNoQuotaMetricsReturned => "Aucun indicateur renvoyé",
        MessageKey::UsageSnapshot => "Aperçu",
        MessageKey::UsageAtRisk => "à risque",
        MessageKey::UsageEmailsHidden => "e-mails masqués",

        // Usage: credit-bank expiry rows and counters
        MessageKey::CreditExpiryUnknown => "expiration inconnue",
        MessageKey::CreditExpiresPrefix => "expire le",
        MessageKey::CreditNearestExpires => "prochaine expiration",
        MessageKey::CreditMoreResetCredits => "autre crédit de réinit.",
        MessageKey::CreditMoreResetCreditsPlural => "autres crédits de réinit.",
        MessageKey::CreditCountSingular => "crédit",
        MessageKey::CreditCountPlural => "crédits",
        MessageKey::CreditAvailableSingular => "disponible",
        MessageKey::CreditAvailablePlural => "disponibles",
        MessageKey::CreditAvailableAcrossAccounts => "disponibles sur les comptes",
        MessageKey::UsageNoResetCredits => "Aucun crédit de réinit.",

        // Usage: the Codex login panel
        MessageKey::CodexLoginTitle => "Connexion Codex",
        MessageKey::CodexLoginImported => "Importé",
        MessageKey::CodexLoginFailed => "Échec",
        MessageKey::CodexLoginRunning => "En cours",
        MessageKey::CodexLoginIdle => "Inactif",
        MessageKey::CodexLoginCancel => "[Annuler]",
        MessageKey::CodexLoginDismiss => "[Fermer]",
        MessageKey::CodexLoginWaiting => "En attente de la sortie codex...",
        MessageKey::CodexLoginImportedPrefix => "Importé :",

        // Usage: the fetching spinner
        // 15 cells. `render_fetching` takes the short branch below width 40 and
        // draws `{spinner} {message}`, so this form must fit `width - 2` at the
        // narrowest width the Usage tab renders at. `Récupération de
        // l'utilisation...` was 32 and clipped with no marker below 34 —
        // exactly the A5 defect the long form had, in the branch that exists to
        // avoid it. Every other language's short form is 15-17 cells.
        MessageKey::UsageFetchingShort => "Récupération...",
        // 31 cells. `render_fetching` takes this branch from width 40, and the
        // line it draws is `{spinner} {message}`, so a 40-cell message needed
        // 42 and was clipped at 40-41 with no marker. Every other language's
        // long form fits inside 38.
        MessageKey::UsageFetchingLong => "Récupération de l'abonnement...",

        // Usage: compact (<48 columns) action-bar labels
        MessageKey::ActionRefreshSyncingShort => "r Sync",
        MessageKey::ActionAddingCodexShort => "a Ajout",
        MessageKey::ActionAddCodexShort => "a Ajout",
        MessageKey::ActionShowEmailsShort => "m Voir",
        MessageKey::ActionHideEmailsShort => "m Masq.",

        // Usage: credential provenance wording
        MessageKey::CredentialSavedActive => "stockage local, connexion Codex actuelle",
        MessageKey::CredentialSaved => "stockage local",
        MessageKey::CredentialManagedByPrefix => "géré par",
        MessageKey::CredentialManagedExternally => "géré en externe",
        MessageKey::UsageCurrentAccount => "Compte actuel",
        MessageKey::UsageManagedByPrefix => "Géré par",
        MessageKey::UsageManagedExternally => "Géré en externe",


    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_languages_have_valid_native_names() {
        for lang in TuiLanguage::ALL {
            assert!(!lang.code().is_empty());
            assert!(!lang.native_name().is_empty());
            assert_eq!(TuiLanguage::from_code(lang.code()), Some(lang));
        }
    }

    /// One concept, one wording per screen.
    ///
    /// The `State` row and the `Health` column hold the same vocabulary and are
    /// visible on the same frame, so a concept translated two ways there reads
    /// as a typo rather than as a deliberate short form. #1367 shipped ko
    /// `한도 부족` (State) against `한도부족` (Health) — the same four syllables
    /// differing only by a space — and ko `알 수 없음` against `미확인` for one
    /// `Unknown`.
    ///
    /// The Health column is `Constraint::Length(8)`, so a *shorter* form there
    /// is allowed and expected; what is not allowed is a form that differs only
    /// cosmetically, or a second word for a concept that fits in both places.
    #[test]
    fn one_concept_is_worded_one_way_per_screen() {
        let normalize = |s: &str| s.replace([' ', '\u{3000}'], "");

        for lang in TuiLanguage::ALL {
            // `Unknown` is short in every language, so State, Health, Capacity
            // and the plan fallback all say the same word.
            let unknown = tr(lang, MessageKey::HealthUnknown);
            for key in [
                MessageKey::StateUnknown,
                MessageKey::LabelUnknown,
                MessageKey::CapacityUnknown,
            ] {
                assert!(
                    tr(lang, key).eq_ignore_ascii_case(unknown),
                    "{}: {key:?} is {:?} but HealthUnknown is {unknown:?}; one `Unknown` \
                     concept, on one frame, must not have two words",
                    lang.code(),
                    tr(lang, key),
                );
            }

            // `Quota low` may be abbreviated for the 8-cell Health column, but
            // the short form must be visibly shorter, not the same word with a
            // space removed.
            let state = tr(lang, MessageKey::StateQuotaLow);
            let health = tr(lang, MessageKey::HealthQuotaLow);
            assert!(
                state.eq_ignore_ascii_case(health) || normalize(state) != normalize(health),
                "{}: StateQuotaLow {state:?} and HealthQuotaLow {health:?} differ only by \
                 whitespace, which reads as a typo; make them identical or genuinely \
                 shorter",
                lang.code(),
            );
        }

        // The credit pool has one name in the space-separated languages: the
        // `Reset Bank` K/V label must reuse a word from the `Credit Bank`
        // heading, not introduce a second noun for the same pool. fr shipped
        // `Réserve` against the heading `Banque de crédits`.
        //
        // Scoped to en/fr on purpose. The CJK catalogs shorten by dropping
        // characters rather than words (ko `크레딧 보관함` -> `초기화권`), so a
        // word-overlap rule does not describe them and is not what those
        // catalogs were reviewed against.
        for lang in [TuiLanguage::En, TuiLanguage::Fr] {
            let heading = tr(lang, MessageKey::HeadingCreditBank).to_lowercase();
            let label = tr(lang, MessageKey::LabelResetBank).to_lowercase();
            let shares_a_word = label.split_whitespace().any(|word| {
                let word = word.trim_matches(|c: char| !c.is_alphanumeric());
                word.chars().count() > 3
                    && heading
                        .split_whitespace()
                        .any(|other| other.starts_with(word) || word.starts_with(other))
            });
            assert!(
                shares_a_word,
                "{}: LabelResetBank {:?} shares no word with HeadingCreditBank {:?}; \
                 the same pool must not have two names on one screen",
                lang.code(),
                tr(lang, MessageKey::LabelResetBank),
                tr(lang, MessageKey::HeadingCreditBank),
            );
        }
    }

    /// The narrow `Client` column names the client program, and the wide one
    /// spells it out, so the short form must fit its column and must not be a
    /// *different word*.
    ///
    /// ko shipped `도구` ("tool"), which is not "client" and does not connect to
    /// the wide layout's `클라이언트`; `클라` is the ordinary Korean clipping and
    /// is 4 cells. ja `ツール` is idiomatic for CLI tooling and stays, so the
    /// prefix rule is asserted only for the languages whose short form is
    /// derived by clipping.
    #[test]
    fn the_short_client_header_fits_and_names_the_client() {
        for lang in TuiLanguage::ALL {
            let short = tr(lang, MessageKey::ColClientShort);
            let width = unicode_width::UnicodeWidthStr::width(short);
            assert!(
                width <= 9,
                "{}: ColClientShort {short:?} is {width} cells; the narrow Sessions \
                 layout grants about 9",
                lang.code(),
            );
        }
        for lang in [
            TuiLanguage::En,
            TuiLanguage::Ko,
            TuiLanguage::ZhCn,
            TuiLanguage::Fr,
        ] {
            let long = tr(lang, MessageKey::ColClient);
            let short = tr(lang, MessageKey::ColClientShort);
            assert!(
                long.to_lowercase().starts_with(&short.to_lowercase()),
                "{}: ColClientShort {short:?} is not a clipping of ColClient {long:?}; \
                 the two layouts must name the same thing",
                lang.code(),
            );
        }
    }

    #[test]
    fn test_all_keys_translate_or_fallback_without_empty() {
        let all_keys = [
            MessageKey::TabOverview,
            MessageKey::TabUsage,
            MessageKey::TabModels,
            MessageKey::TabDaily,
            MessageKey::TabHourly,
            MessageKey::TabMinutely,
            MessageKey::TabMonthly,
            MessageKey::TabSessions,
            MessageKey::TabProjects,
            MessageKey::TabStats,
            MessageKey::TabAgents,
            MessageKey::TabOverviewShort,
            MessageKey::TabUsageShort,
            MessageKey::TabModelsShort,
            MessageKey::TabDailyShort,
            MessageKey::TabHourlyShort,
            MessageKey::TabMinutelyShort,
            MessageKey::TabMonthlyShort,
            MessageKey::TabSessionsShort,
            MessageKey::TabProjectsShort,
            MessageKey::TabStatsShort,
            MessageKey::TabAgentsShort,
            MessageKey::ColDate,
            MessageKey::ColCost,
            MessageKey::ColTokens,
            MessageKey::ColModel,
            MessageKey::ColClient,
            MessageKey::ColClientShort,
            MessageKey::ColProvider,
            MessageKey::ColSource,
            MessageKey::ColMessages,
            MessageKey::ColMessagesShort,
            MessageKey::ColInput,
            MessageKey::ColOutput,
            MessageKey::ColCacheRead,
            MessageKey::ColCacheWrite,
            MessageKey::ColSession,
            MessageKey::ColProject,
            MessageKey::ColDuration,
            MessageKey::ColWorkspace,
            MessageKey::ColTotal,
            MessageKey::ColTurn,
            MessageKey::ColMonth,
            MessageKey::ColHour,
            MessageKey::ColMinute,
            MessageKey::ColAgent,
            MessageKey::ColSessions,
            MessageKey::ColSources,
            MessageKey::ColModels,
            MessageKey::ColRank,
            MessageKey::ColMsPer1k,
            MessageKey::ColCostPer1M,
            MessageKey::ColLastActive,
            MessageKey::OverviewTopModels,
            MessageKey::OverviewModelsByTokens,
            MessageKey::OverviewModelsByCost,
            MessageKey::OverviewTotal,
            MessageKey::EmptyNoUsageData,
            MessageKey::EmptyNoModelDetailsDay,
            MessageKey::FooterTokens,
            MessageKey::CountModels,
            MessageKey::CountAgents,
            MessageKey::CountDay,
            MessageKey::CountDays,
            MessageKey::CountHours,
            MessageKey::CountMinutes,
            MessageKey::CountMonths,
            MessageKey::CountSessions,
            MessageKey::CountProjects,
            MessageKey::SortLabel,
            MessageKey::SortDate,
            MessageKey::SortCost,
            MessageKey::SortTokens,
            MessageKey::HelpScroll,
            MessageKey::HelpSort,
            MessageKey::HelpBack,
            MessageKey::HelpDetails,
            MessageKey::HelpToday,
            MessageKey::HelpProfile,
            MessageKey::HelpSources,
            MessageKey::HelpLanguage,
            MessageKey::HelpRefresh,
            MessageKey::HelpQuit,
            MessageKey::DialogCloseHint,
            MessageKey::DialogFilterPlaceholder,
            MessageKey::DialogFilterLabel,
            MessageKey::DialogNoResults,
            MessageKey::LanguageDialogTitle,
            MessageKey::LanguageDialogHint,
            MessageKey::ClientDialogTitle,
            MessageKey::ClientDialogHint,
            MessageKey::GroupByDialogTitle,
            MessageKey::GroupByDialogHint,
            MessageKey::GroupByModelLabel,
            MessageKey::GroupByModelDesc,
            MessageKey::GroupByClientModelLabel,
            MessageKey::GroupByClientModelDesc,
            MessageKey::GroupByClientProviderModelLabel,
            MessageKey::GroupByClientProviderModelDesc,
            MessageKey::GroupByWorkspaceModelLabel,
            MessageKey::GroupByWorkspaceModelDesc,
            MessageKey::GroupBySessionLabel,
            MessageKey::GroupBySessionDesc,
            MessageKey::GroupByClientSessionLabel,
            MessageKey::GroupByClientSessionDesc,
            MessageKey::ConfirmYes,
            MessageKey::ConfirmNo,
            MessageKey::StatusLoadedFromCache,
            MessageKey::StatusRefreshInProgress,
            MessageKey::StatusLanguageChanged,
            MessageKey::ColCacheHit,
            MessageKey::TitleDailyUsage,
            MessageKey::TitleMonthlyUsage,
            MessageKey::TitleHourlyUsage,
            MessageKey::TitleHourlyProfile,
            MessageKey::TitleMinutelyUsage,
            MessageKey::TitleUsageSummary,
            MessageKey::TitleAccounts,
            MessageKey::TitleSelectedAccount,
            MessageKey::TitleContributionGraph,
            MessageKey::TitleDayBreakdown,
            MessageKey::TitleDailyDetail,
            MessageKey::TitleDailyDetailPrefix,
            MessageKey::TitleDailyBreakdown,
            MessageKey::TitleDailyBreakdownPrefix,
            MessageKey::ChartTokensPerDay,
            MessageKey::ChartTokens,
            MessageKey::EmptyNoDailyData,
            MessageKey::EmptyNoMonthlyData,
            MessageKey::EmptyNoDailyDataMonth,
            MessageKey::EmptyNoHourlyData,
            MessageKey::EmptyNoMinutelyData,
            MessageKey::EmptyNoSessionData,
            MessageKey::EmptyNoProjectData,
            MessageKey::EmptyNoDataAvailable,
            MessageKey::EmptyNoDataForDay,
            MessageKey::EmptyNoBreakdownAvailable,
            MessageKey::EmptyNoAgentCodex,
            MessageKey::EmptyNoAgentMixed,
            MessageKey::StatusRefreshingBackground,
            MessageKey::StatusLastUpdated,
            MessageKey::StatusLocal,
            MessageKey::StatusDevice,
            MessageKey::StatusAllDevices,
            MessageKey::StatsFavoriteModel,
            MessageKey::StatsFavoriteModelShort,
            MessageKey::StatsTotalTokens,
            MessageKey::StatsTokensShort,
            MessageKey::StatsSessions,
            MessageKey::StatsTotalCost,
            MessageKey::StatsCostShort,
            MessageKey::StatsCurrentStreak,
            MessageKey::StatsStreakShort,
            MessageKey::StatsLongestStreak,
            MessageKey::StatsLongestStreakShort,
            MessageKey::StatsActiveDays,
            MessageKey::StatsActiveShort,
            MessageKey::StatsLess,
            MessageKey::StatsMore,
            MessageKey::DialogCurrentLabel,
            MessageKey::DialogCancel,
            MessageKey::DialogTargetLabel,
            MessageKey::DialogEffectLabel,
            MessageKey::PhaseInitializing,
            MessageKey::PhaseScanningSessions,
            MessageKey::PhaseLoadingPricing,
            MessageKey::PhaseFinalizingReport,
            MessageKey::PhaseComplete,
            MessageKey::PhaseLoadingData,
            MessageKey::LabelError,
            MessageKey::ProfileWhenYouWorkMost,
            MessageKey::ProfileMostProductiveDay,
            MessageKey::ProfilePeakHour,
            MessageKey::ProfileLegend,
            MessageKey::ProfileLow,
            MessageKey::ProfileHigh,
            MessageKey::ProfileTotalTokens,
            MessageKey::ProfileTotalCost,
            MessageKey::PeriodMorning,
            MessageKey::PeriodDaytime,
            MessageKey::PeriodEvening,
            MessageKey::PeriodNight,
            MessageKey::WeekdayMonday,
            MessageKey::WeekdayTuesday,
            MessageKey::WeekdayWednesday,
            MessageKey::WeekdayThursday,
            MessageKey::WeekdayFriday,
            MessageKey::WeekdaySaturday,
            MessageKey::WeekdaySunday,
            MessageKey::ProfileSwitchHint,
            MessageKey::StatusLanguageSaveFailed,
            MessageKey::StatusSyncingUsage,
            MessageKey::StatusCodexLogin,
            MessageKey::StatusNoData,
            MessageKey::StatusNotLoaded,
            MessageKey::StatusSavedSingular,
            MessageKey::StatusSavedPlural,
            MessageKey::StatusManagedSingular,
            MessageKey::StatusManagedPlural,
            MessageKey::StatusIssueSingular,
            MessageKey::StatusIssuePlural,
            MessageKey::ActionRefresh,
            MessageKey::ActionRefreshSyncing,
            MessageKey::ActionAddCodex,
            MessageKey::ActionAddingCodex,
            MessageKey::ActionShowEmails,
            MessageKey::ActionHideEmails,
            MessageKey::ActionReset,
            MessageKey::HeadingAttention,
            MessageKey::HeadingDiagnostics,
            MessageKey::HeadingProviders,
            MessageKey::HeadingLimits,
            MessageKey::HeadingActions,
            MessageKey::HeadingCreditBank,
            MessageKey::LabelStatus,
            MessageKey::LabelEmail,
            MessageKey::LabelCredential,
            MessageKey::LabelCredits,
            MessageKey::LabelResetBank,
            MessageKey::ColAccount,
            MessageKey::ColPlan,
            MessageKey::ColAuth,
            MessageKey::ColHealth,
            MessageKey::ColLimit,
            MessageKey::ColReset,
            MessageKey::UsageAccountOrStatus,
            MessageKey::UsageEmptyNoData,
            MessageKey::UsageEmptyNotLoadedTitle,
            MessageKey::UsageEmptyNotLoadedHint,
            MessageKey::UsageFetchFailed,
            MessageKey::UsageEmptyNoSubscriptionData,
            MessageKey::UsageNoAttentionNeeded,
            MessageKey::StateSwitchRecommended,
            MessageKey::StateReady,
            MessageKey::StateReadyWithWarnings,
            MessageKey::StateQuotaLow,
            MessageKey::StateUnknown,
            MessageKey::HealthReady,
            MessageKey::HealthWatch,
            MessageKey::HealthQuotaLow,
            MessageKey::HealthUnknown,
            MessageKey::LabelState,
            MessageKey::LabelActiveAccount,
            MessageKey::LabelCapacity,
            MessageKey::LabelFallback,
            MessageKey::LabelNextReset,
            MessageKey::LabelAction,
            MessageKey::UsageNoActiveAccount,
            MessageKey::UsageNoReadyFallback,
            MessageKey::UsageNoResetData,
            MessageKey::UsagePercentLeft,
            MessageKey::CapacityReady,
            MessageKey::CapacityWatch,
            MessageKey::CapacityCritical,
            MessageKey::CapacityUnknown,
            MessageKey::ActionChooseActive,
            MessageKey::ActionRefreshUnknownLimits,
            MessageKey::ActionKeepCurrent,
            MessageKey::ActionMonitorQuota,
            MessageKey::ActionUsePrefix,
            MessageKey::ActionWaitForReset,
            MessageKey::ActionRefreshActive,
            MessageKey::AuthActive,
            MessageKey::AuthSaved,
            MessageKey::AuthManaged,
            MessageKey::StateAuthenticated,
            MessageKey::LabelUnknown,
            MessageKey::UsageMoreIssues,
            MessageKey::UsageMoreIssuesPlural,
            MessageKey::UsageMoreAtRisk,
            MessageKey::UsageMoreAtRiskPlural,
            MessageKey::ButtonUseAccount,
            MessageKey::ButtonRemove,
            MessageKey::ButtonReset,
            MessageKey::UsageNoLimits,
            MessageKey::UsageNoQuotaMetrics,
            MessageKey::UsageNoQuotaMetricsReturned,
            MessageKey::UsageSnapshot,
            MessageKey::UsageAtRisk,
            MessageKey::UsageEmailsHidden,
            MessageKey::CreditExpiryUnknown,
            MessageKey::CreditExpiresPrefix,
            MessageKey::CreditNearestExpires,
            MessageKey::CreditMoreResetCredits,
            MessageKey::CreditMoreResetCreditsPlural,
            MessageKey::CreditCountSingular,
            MessageKey::CreditCountPlural,
            MessageKey::CreditAvailableSingular,
            MessageKey::CreditAvailablePlural,
            MessageKey::CreditAvailableAcrossAccounts,
            MessageKey::UsageNoResetCredits,
            MessageKey::CodexLoginTitle,
            MessageKey::CodexLoginImported,
            MessageKey::CodexLoginFailed,
            MessageKey::CodexLoginRunning,
            MessageKey::CodexLoginIdle,
            MessageKey::CodexLoginCancel,
            MessageKey::CodexLoginDismiss,
            MessageKey::CodexLoginWaiting,
            MessageKey::CodexLoginImportedPrefix,
            MessageKey::UsageFetchingShort,
            MessageKey::UsageFetchingLong,
            MessageKey::ActionRefreshSyncingShort,
            MessageKey::ActionAddingCodexShort,
            MessageKey::ActionAddCodexShort,
            MessageKey::ActionShowEmailsShort,
            MessageKey::ActionHideEmailsShort,
            MessageKey::CredentialSavedActive,
            MessageKey::CredentialSaved,
            MessageKey::CredentialManagedByPrefix,
            MessageKey::CredentialManagedExternally,
            MessageKey::UsageCurrentAccount,
            MessageKey::UsageManagedByPrefix,
            MessageKey::UsageManagedExternally,
        ];

        for lang in TuiLanguage::ALL {
            for &key in &all_keys {
                let text = tr(lang, key);
                assert!(!text.is_empty(), "key {:?} in {:?} is empty", key, lang);
                // Ensure display width is non-zero
                assert!(
                    unicode_width::UnicodeWidthStr::width(text) > 0,
                    "width of key {:?} in {:?} is 0",
                    key,
                    lang
                );
            }
        }
    }
}
