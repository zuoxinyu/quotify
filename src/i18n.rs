use parking_lot::RwLock;
use std::{borrow::Cow, sync::OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    SimplifiedChinese,
    TraditionalChinese,
    Japanese,
}

impl Language {
    fn index(self) -> usize {
        match self {
            Self::English => 0,
            Self::SimplifiedChinese => 1,
            Self::TraditionalChinese => 2,
            Self::Japanese => 3,
        }
    }

    pub fn text(self, key: Text) -> &'static str {
        translations(key)[self.index()]
    }

    fn component_locale(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
            // gpui-component 0.5.1 names its Traditional Chinese catalog zh-HK.
            Self::TraditionalChinese => "zh-HK",
            // 0.5.1 has no Japanese catalog yet and falls back to English.
            Self::Japanese => "ja",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageSetting {
    System,
    English,
    SimplifiedChinese,
    TraditionalChinese,
    Japanese,
}

impl LanguageSetting {
    pub const ALL: [Self; 5] = [
        Self::System,
        Self::English,
        Self::SimplifiedChinese,
        Self::TraditionalChinese,
        Self::Japanese,
    ];

    pub fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "english" => Self::English,
            "zh-cn" | "zh-hans" | "simplified_chinese" => Self::SimplifiedChinese,
            "zh-tw" | "zh-hant" | "traditional_chinese" => Self::TraditionalChinese,
            "ja" | "ja-jp" | "japanese" => Self::Japanese,
            _ => Self::System,
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
            Self::TraditionalChinese => "zh-TW",
            Self::Japanese => "ja",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::English => "English",
            Self::SimplifiedChinese => "简体中文",
            Self::TraditionalChinese => "繁體中文",
            Self::Japanese => "日本語",
        }
    }

    pub fn effective(self) -> Language {
        match self {
            Self::System => system_language(),
            Self::English => Language::English,
            Self::SimplifiedChinese => Language::SimplifiedChinese,
            Self::TraditionalChinese => Language::TraditionalChinese,
            Self::Japanese => Language::Japanese,
        }
    }
}

static CURRENT_LANGUAGE: OnceLock<RwLock<Language>> = OnceLock::new();

fn current_language_cell() -> &'static RwLock<Language> {
    CURRENT_LANGUAGE.get_or_init(|| RwLock::new(system_language()))
}

pub fn set_current_language(setting: &str) -> Language {
    let language = LanguageSetting::from_config(setting).effective();
    *current_language_cell().write() = language;
    gpui_component::set_locale(language.component_locale());
    language
}

pub fn current_language() -> Language {
    *current_language_cell().read()
}

pub fn text(key: Text) -> &'static str {
    current_language().text(key)
}

pub fn system_language() -> Language {
    system_locale_name()
        .as_deref()
        .map(language_from_locale)
        .unwrap_or(Language::English)
}

#[cfg(target_os = "windows")]
fn system_locale_name() -> Option<String> {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;

    let mut buffer = [0_u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if length <= 1 {
        return None;
    }
    String::from_utf16(&buffer[..length as usize - 1]).ok()
}

#[cfg(not(target_os = "windows"))]
fn system_locale_name() -> Option<String> {
    std::env::var("LANG").ok()
}

fn language_from_locale(locale: &str) -> Language {
    let normalized = locale.trim().replace('_', "-").to_ascii_lowercase();
    if normalized == "zh"
        || normalized.starts_with("zh-cn")
        || normalized.starts_with("zh-sg")
        || normalized.contains("hans")
    {
        Language::SimplifiedChinese
    } else if normalized.starts_with("zh-tw")
        || normalized.starts_with("zh-hk")
        || normalized.starts_with("zh-mo")
        || normalized.contains("hant")
    {
        Language::TraditionalChinese
    } else if normalized == "ja" || normalized.starts_with("ja-") {
        Language::Japanese
    } else {
        Language::English
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    About,
    Settings,
    Back,
    AboutQuotify,
    RefreshUsage,
    JustNow,
    WelcomeTitle,
    WelcomeSubtitle,
    LocalOnlyDiscovery,
    LocalOnlyDiscoveryDescription,
    YouStayInControl,
    YouStayInControlDescription,
    NotNow,
    AllowAndScan,
    ScanningAgents,
    ScanFewSeconds,
    NoReadyAgentsOnboarding,
    ScanComplete,
    Continue,
    NoEnabledProviders,
    HideResetDetails,
    ShowResetDetails,
    BudgetUnavailable,
    WaitingForLogin,
    RetryLogin,
    LogIn,
    LoginFailed,
    HideUsageHistogram,
    ShowUsageHistogram,
    HistogramZero,
    HistogramUnavailable,
    HistogramCaption,
    ResetCredit,
    Expires,
    NoResetCreditDetails,
    NoExpiration,
    Resetting,
    Version,
    Author,
    CheckForUpdates,
    CheckNow,
    CheckingUpdates,
    AppUpToDate,
    ViewReleasePage,
    UpdateCheckFailed,
    GeneralSettings,
    Language,
    LanguageDescription,
    Theme,
    ThemeDescription,
    System,
    Dark,
    Light,
    Backdrop,
    BackdropDescription,
    None,
    AutomaticWebViewLogin,
    AutomaticWebViewLoginDescription,
    LocalAgentDiscovery,
    LocalAgentDiscoveryDescription,
    ScanNow,
    NoReadyLocalAgents,
    Enabled,
    StartWithWindows,
    StartWithWindowsDescription,
    RefreshInterval,
    NetworkProxy,
    Notifications,
    WindowsNotifications,
    WindowsNotificationsDescription,
    MonthlyQuotaReset,
    MonthlyQuotaResetDescription,
    WeeklyQuotaReset,
    WeeklyQuotaResetDescription,
    FiveHourQuotaReset,
    FiveHourQuotaResetDescription,
    UsageThreshold,
    UsageThresholdDescription,
    SilentRefreshFailures,
    SilentRefreshFailuresDescription,
    ProviderSettings,
    SearchProviders,
    OpenConfigFile,
    OpenLogs,
    CreateDiagnosticReport,
    PrimaryProvider,
    Primary,
    SetAsPrimary,
    EnableProvider,
    ApiKey,
    SessionKey,
    AccessToken,
    AuthFilePath,
    WorkspaceId,
    AuthCookie,
    ServiceToken,
    CookieHeader,
    ApiKeyToken,
    PasteProviderCredential,
    BaseUrl,
    DeploymentModelName,
    ApiBudget30d,
    ApiBudgetPlaceholder,
    BedrockDescription,
    BudgetDescription,
    BedrockEnvironmentOverride,
    TestProvider,
    TestPassed,
    TestFailed,
    FetchingUsage,
    ProviderNoUsage,
    ProviderCouldNotCreate,
    BudgetWindowMissing,
    BudgetPositive,
    ShowDetails,
    RefreshNow,
    Quit,
    PleaseLoginToContinue,
    QuotaMonitor,
    AllProviders,
    NoUsageData,
    Error,
    Max,
}

fn translations(key: Text) -> [&'static str; 4] {
    use Text::*;
    match key {
        About => ["About", "关于", "關於", "情報"],
        Settings => ["Settings", "设置", "設定", "設定"],
        Back => ["Back", "返回", "返回", "戻る"],
        AboutQuotify => [
            "About Quotify",
            "关于 Quotify",
            "關於 Quotify",
            "Quotify について",
        ],
        RefreshUsage => ["Refresh usage", "刷新用量", "重新整理用量", "使用量を更新"],
        JustNow => ["just now", "刚刚", "剛剛", "たった今"],
        WelcomeTitle => [
            "Welcome to Quotify",
            "欢迎使用 Quotify",
            "歡迎使用 Quotify",
            "Quotify へようこそ",
        ],
        WelcomeSubtitle => [
            "Find your coding agents and get set up faster.",
            "发现本机的编码 Agent，快速完成设置。",
            "尋找本機的程式設計 Agent，快速完成設定。",
            "ローカルの Agent を検出してすばやく設定。",
        ],
        LocalOnlyDiscovery => [
            "Local-only discovery",
            "仅在本机扫描",
            "僅在本機掃描",
            "ローカルのみで検出",
        ],
        LocalOnlyDiscoveryDescription => [
            "Checks known credential locations, environment variables, and installed CLI commands.",
            "检查已知凭据位置、环境变量和已安装的 CLI 命令。",
            "檢查已知憑證位置、環境變數和已安裝的 CLI 命令。",
            "既知の認証情報、環境変数、インストール済み CLI を確認します。",
        ],
        YouStayInControl => [
            "You stay in control",
            "一切由你掌控",
            "一切由你掌控",
            "常にユーザーが制御",
        ],
        YouStayInControlDescription => [
            "Nothing is uploaded. Detected providers are enabled so Quotify can display their usage.",
            "不会上传任何数据。检测到的服务商会被启用，以便 Quotify 显示其用量。",
            "不會上傳任何資料。偵測到的供應商會被啟用，讓 Quotify 顯示其用量。",
            "データはアップロードされません。検出したプロバイダーを有効化し、Quotify に使用量を表示します。",
        ],
        NotNow => ["Not now", "暂不", "暫不", "後で"],
        AllowAndScan => [
            "Allow & Scan",
            "允许并扫描",
            "允許並掃描",
            "許可してスキャン",
        ],
        ScanningAgents => [
            "Scanning for local agents...",
            "正在扫描本地 Agent…",
            "正在掃描本機 Agent…",
            "ローカルエージェントをスキャン中…",
        ],
        ScanFewSeconds => [
            "This usually takes only a few seconds.",
            "通常只需要几秒钟。",
            "通常只需要幾秒鐘。",
            "通常は数秒で完了します。",
        ],
        NoReadyAgentsOnboarding => [
            "No ready agents were found. You can configure providers manually in Settings.",
            "未找到可用的 Agent。你可以稍后在设置中手动配置服务商。",
            "找不到可用的 Agent。你可以稍後在設定中手動設定供應商。",
            "利用可能なエージェントが見つかりませんでした。設定からプロバイダーを手動で構成できます。",
        ],
        ScanComplete => ["Scan complete", "扫描完成", "掃描完成", "スキャン完了"],
        Continue => ["Continue", "继续", "繼續", "続行"],
        NoEnabledProviders => [
            "No enabled providers. Configure credentials to enable cards.",
            "没有已启用的服务商。请配置凭据以显示卡片。",
            "沒有已啟用的供應商。請設定憑證以顯示卡片。",
            "有効なプロバイダーがありません。認証情報を設定してカードを有効にしてください。",
        ],
        HideResetDetails => [
            "Hide reset credit expiration details",
            "隐藏重置额度到期详情",
            "隱藏重設額度到期詳情",
            "リセットクレジットの有効期限を隠す",
        ],
        ShowResetDetails => [
            "Show reset credit expiration details",
            "显示重置额度到期详情",
            "顯示重設額度到期詳情",
            "リセットクレジットの有効期限を表示",
        ],
        BudgetUnavailable => [
            "Budget unavailable",
            "预算不可用",
            "預算不可用",
            "予算を取得できません",
        ],
        WaitingForLogin => [
            "Waiting for login...",
            "正在等待登录…",
            "正在等待登入…",
            "ログインを待機中…",
        ],
        RetryLogin => ["Retry login", "重试登录", "重試登入", "ログインを再試行"],
        LogIn => ["Log in", "登录", "登入", "ログイン"],
        LoginFailed => [
            "Login failed",
            "登录失败",
            "登入失敗",
            "ログインに失敗しました",
        ],
        HideUsageHistogram => [
            "Hide 7-day usage histogram",
            "隐藏 7 天用量直方图",
            "隱藏 7 天用量直方圖",
            "7日間の使用量ヒストグラムを隠す",
        ],
        ShowUsageHistogram => [
            "Show 7-day usage histogram",
            "显示 7 天用量直方图",
            "顯示 7 天用量直方圖",
            "7日間の使用量ヒストグラムを表示",
        ],
        HistogramZero => [
            "Available latest samples round to 0%",
            "最新可用样本四舍五入后为 0%",
            "最新可用樣本四捨五入後為 0%",
            "最新のサンプルは丸めると 0% です",
        ],
        HistogramUnavailable => [
            "Latest samples unavailable for these buckets",
            "这些时间段没有最新样本",
            "這些時段沒有最新樣本",
            "これらの期間には最新サンプルがありません",
        ],
        HistogramCaption => [
            "Latest sample per rolling 24h · shared relative scale",
            "每个滚动 24 小时的最新样本 · 共用相对刻度",
            "每個滾動 24 小時的最新樣本 · 共用相對刻度",
            "各24時間枠の最新サンプル · 共通の相対スケール",
        ],
        ResetCredit => ["Reset credit", "重置额度", "重設額度", "リセットクレジット"],
        Expires => ["Expires", "到期时间", "到期時間", "有効期限"],
        NoResetCreditDetails => [
            "No reset credit details returned.",
            "未返回重置额度详情。",
            "未傳回重設額度詳情。",
            "リセットクレジットの詳細がありません。",
        ],
        NoExpiration => ["No expiration", "永不过期", "永不過期", "有効期限なし"],
        Resetting => ["resetting", "正在重置", "正在重設", "リセット中"],
        Version => ["Version", "版本", "版本", "バージョン"],
        Author => ["Author", "作者", "作者", "作者"],
        CheckForUpdates => [
            "Check for Updates",
            "检查更新",
            "檢查更新",
            "アップデートを確認",
        ],
        CheckNow => ["Check now", "立即检查", "立即檢查", "今すぐ確認"],
        CheckingUpdates => [
            "Checking for updates...",
            "正在检查更新…",
            "正在檢查更新…",
            "アップデートを確認中…",
        ],
        AppUpToDate => [
            "App is up to date.",
            "当前已是最新版本。",
            "目前已是最新版本。",
            "最新バージョンです。",
        ],
        ViewReleasePage => [
            "View Release Page",
            "查看发布页面",
            "查看發行頁面",
            "リリースページを表示",
        ],
        UpdateCheckFailed => [
            "Update check failed",
            "检查更新失败",
            "檢查更新失敗",
            "アップデート確認に失敗しました",
        ],
        GeneralSettings => ["General Settings", "通用设置", "一般設定", "一般設定"],
        Language => ["Language", "语言", "語言", "言語"],
        LanguageDescription => [
            "Choose the display language",
            "选择显示语言",
            "選擇顯示語言",
            "表示言語を選択",
        ],
        Theme => ["Theme", "主题", "主題", "テーマ"],
        ThemeDescription => [
            "Configure app color palette",
            "配置应用配色",
            "設定應用程式配色",
            "アプリの配色を設定",
        ],
        System => ["system", "系统", "系統", "システム"],
        Dark => ["dark", "深色", "深色", "ダーク"],
        Light => ["light", "浅色", "淺色", "ライト"],
        Backdrop => ["Backdrop", "背景材质", "背景材質", "背景素材"],
        BackdropDescription => [
            "Windows material effect",
            "Windows 材质效果",
            "Windows 材質效果",
            "Windows のマテリアル効果",
        ],
        None => ["None", "无", "無", "なし"],
        AutomaticWebViewLogin => [
            "Automatic WebView Login",
            "自动 WebView 登录",
            "自動 WebView 登入",
            "WebView 自動ログイン",
        ],
        AutomaticWebViewLoginDescription => [
            "Open WebView when authentication fails",
            "认证失败时打开 WebView",
            "驗證失敗時開啟 WebView",
            "認証失敗時に WebView を開く",
        ],
        LocalAgentDiscovery => [
            "Local Agent Discovery",
            "本地 Agent 扫描",
            "本機 Agent 掃描",
            "ローカルエージェント検出",
        ],
        LocalAgentDiscoveryDescription => [
            "Check known local auth locations and installed agent CLIs",
            "检测本地凭据和 Agent CLI",
            "偵測本機憑證與 Agent CLI",
            "既知の認証情報と Agent CLI を検出",
        ],
        ScanNow => ["Scan now", "扫描", "掃描", "スキャン"],
        NoReadyLocalAgents => [
            "No ready local agents were found.",
            "未找到可用的本地 Agent。",
            "找不到可用的本機 Agent。",
            "利用可能なローカルエージェントが見つかりませんでした。",
        ],
        Enabled => ["Enabled", "已启用", "已啟用", "有効化"],
        StartWithWindows => [
            "Start with Windows",
            "随 Windows 启动",
            "隨 Windows 啟動",
            "Windows と同時に起動",
        ],
        StartWithWindowsDescription => [
            "Launch Quotify when you sign in",
            "登录 Windows 时启动 Quotify",
            "登入 Windows 時啟動 Quotify",
            "Windows サインイン時に Quotify を起動",
        ],
        RefreshInterval => ["Refresh Interval", "刷新间隔", "重新整理間隔", "更新間隔"],
        NetworkProxy => [
            "Network Proxy",
            "网络代理",
            "網路代理",
            "ネットワークプロキシ",
        ],
        Notifications => ["Notifications", "通知", "通知", "通知"],
        WindowsNotifications => [
            "Windows Notifications",
            "Windows 通知",
            "Windows 通知",
            "Windows 通知",
        ],
        WindowsNotificationsDescription => [
            "Disabled by default; respects Windows quiet hours",
            "默认关闭；遵循 Windows 免打扰时段",
            "預設關閉；遵循 Windows 勿擾時段",
            "既定では無効。Windows の集中モードに従います",
        ],
        MonthlyQuotaReset => [
            "Monthly quota reset",
            "月额度重置",
            "月額度重設",
            "月間クォータのリセット",
        ],
        MonthlyQuotaResetDescription => [
            "After a provider's monthly quota resets",
            "服务商月额度重置后通知",
            "供應商月額度重設後通知",
            "プロバイダーの月間クォータがリセットされた後",
        ],
        WeeklyQuotaReset => [
            "Weekly quota reset",
            "周额度重置",
            "週額度重設",
            "週間クォータのリセット",
        ],
        WeeklyQuotaResetDescription => [
            "After a provider's weekly quota resets",
            "服务商周额度重置后通知",
            "供應商週額度重設後通知",
            "プロバイダーの週間クォータがリセットされた後",
        ],
        FiveHourQuotaReset => [
            "5-hour quota reset",
            "5 小时额度重置",
            "5 小時額度重設",
            "5時間クォータのリセット",
        ],
        FiveHourQuotaResetDescription => [
            "Session and rolling 5-hour windows",
            "Session 和滚动 5 小时窗口",
            "Session 和滾動 5 小時視窗",
            "Session とローリング5時間枠",
        ],
        UsageThreshold => [
            "Usage threshold",
            "用量阈值",
            "用量閾值",
            "使用量のしきい値",
        ],
        UsageThresholdDescription => [
            "Once when usage crosses the threshold",
            "用量超过阈值时通知一次",
            "用量超過閾值時通知一次",
            "使用量がしきい値を超えたときに一度通知",
        ],
        SilentRefreshFailures => [
            "Silent refresh failures",
            "静默刷新失败",
            "靜默重新整理失敗",
            "バックグラウンド更新の失敗",
        ],
        SilentRefreshFailuresDescription => [
            "When an automatic refresh starts failing",
            "自动刷新开始失败时通知",
            "自動重新整理開始失敗時通知",
            "自動更新が失敗し始めたとき",
        ],
        ProviderSettings => [
            "Provider Settings",
            "服务商设置",
            "供應商設定",
            "プロバイダー設定",
        ],
        SearchProviders => [
            "Search providers",
            "搜索服务商",
            "搜尋供應商",
            "プロバイダーを検索",
        ],
        OpenConfigFile => [
            "Open config file",
            "打开配置文件",
            "開啟設定檔",
            "設定ファイルを開く",
        ],
        OpenLogs => ["Open logs", "打开日志", "開啟記錄", "ログを開く"],
        CreateDiagnosticReport => [
            "Create diagnostic report",
            "创建诊断报告",
            "建立診斷報告",
            "診断レポートを作成",
        ],
        PrimaryProvider => [
            "Primary Provider",
            "主要服务商",
            "主要供應商",
            "プライマリプロバイダー",
        ],
        Primary => ["Primary", "主要", "主要", "プライマリ"],
        SetAsPrimary => ["Set as Primary", "设为主要", "設為主要", "プライマリに設定"],
        EnableProvider => [
            "Enable Provider",
            "启用服务商",
            "啟用供應商",
            "プロバイダーを有効化",
        ],
        ApiKey => ["API Key", "API 密钥", "API 金鑰", "API キー"],
        SessionKey => [
            "Session Key",
            "Session 密钥",
            "Session 金鑰",
            "Session キー",
        ],
        AccessToken => ["Access Token", "访问令牌", "存取權杖", "アクセストークン"],
        AuthFilePath => [
            "Auth File Path",
            "认证文件路径",
            "驗證檔案路徑",
            "認証ファイルのパス",
        ],
        WorkspaceId => ["Workspace ID", "工作区 ID", "工作區 ID", "Workspace ID"],
        AuthCookie => ["Auth Cookie", "认证 Cookie", "驗證 Cookie", "認証 Cookie"],
        ServiceToken => ["Service Token", "服务令牌", "服務權杖", "サービストークン"],
        CookieHeader => [
            "Cookie Header",
            "Cookie 请求头",
            "Cookie 標頭",
            "Cookie ヘッダー",
        ],
        ApiKeyToken => [
            "API Key / Token",
            "API 密钥 / 令牌",
            "API 金鑰 / 權杖",
            "API キー / トークン",
        ],
        PasteProviderCredential => [
            "Paste provider credential",
            "粘贴服务商凭据",
            "貼上供應商憑證",
            "プロバイダーの認証情報を貼り付け",
        ],
        BaseUrl => ["Base URL", "基础 URL", "基礎 URL", "ベース URL"],
        DeploymentModelName => [
            "Deployment / Model Name",
            "部署 / 模型名称",
            "部署 / 模型名稱",
            "デプロイ / モデル名",
        ],
        ApiBudget30d => [
            "30-Day API Budget (USD)",
            "30 天 API 预算（USD）",
            "30 天 API 預算（USD）",
            "30日間の API 予算（USD）",
        ],
        ApiBudgetPlaceholder => [
            "e.g. 100; leave empty to disable",
            "例如 100；留空则禁用",
            "例如 100；留空則停用",
            "例: 100（空欄で無効）",
        ],
        BedrockDescription => [
            "Uses the AWS CLI credential chain and Cost Explorer.",
            "使用 AWS CLI 凭据链和 Cost Explorer。",
            "使用 AWS CLI 憑證鏈和 Cost Explorer。",
            "AWS CLI の認証情報チェーンと Cost Explorer を使用します。",
        ],
        BudgetDescription => [
            "Uses the latest 30 complete UTC days of USD spend.",
            "使用最近 30 个完整 UTC 日的美元费用。",
            "使用最近 30 個完整 UTC 日的美元費用。",
            "直近30日間（UTC）の USD 支出を使用します。",
        ],
        BedrockEnvironmentOverride => [
            "CODEXBAR_BEDROCK_BUDGET is set and remains active until the environment variable is removed.",
            "已设置 CODEXBAR_BEDROCK_BUDGET；删除该环境变量前它会一直生效。",
            "已設定 CODEXBAR_BEDROCK_BUDGET；移除該環境變數前會持續生效。",
            "CODEXBAR_BEDROCK_BUDGET が設定されています。環境変数を削除するまで有効です。",
        ],
        TestProvider => [
            "Test Provider",
            "测试服务商",
            "測試供應商",
            "プロバイダーをテスト",
        ],
        TestPassed => [
            "Test passed",
            "测试通过",
            "測試通過",
            "テストに成功しました",
        ],
        TestFailed => [
            "Test failed",
            "测试失败",
            "測試失敗",
            "テストに失敗しました",
        ],
        FetchingUsage => [
            "Fetching usage with current provider settings...",
            "正在使用当前服务商设置获取用量…",
            "正在使用目前供應商設定取得用量…",
            "現在のプロバイダー設定で使用量を取得中…",
        ],
        ProviderNoUsage => [
            "Provider responded, but no usage windows or credits were returned.",
            "服务商已响应，但未返回用量窗口或余额。",
            "供應商已回應，但未傳回用量視窗或餘額。",
            "プロバイダーは応答しましたが、使用量枠やクレジットが返されませんでした。",
        ],
        ProviderCouldNotCreate => [
            "Provider could not be created from the current settings",
            "无法根据当前设置创建服务商",
            "無法根據目前設定建立供應商",
            "現在の設定からプロバイダーを作成できませんでした",
        ],
        BudgetWindowMissing => [
            "A 30-day budget is configured, but this provider did not return a 30-day USD spend window",
            "已配置 30 天预算，但该服务商未返回 30 天美元费用窗口",
            "已設定 30 天預算，但該供應商未傳回 30 天美元費用視窗",
            "30日間の予算が設定されていますが、このプロバイダーは30日間の USD 支出枠を返しませんでした",
        ],
        BudgetPositive => [
            "Enter a positive USD amount, or leave the field empty.",
            "请输入正数美元金额，或将字段留空。",
            "請輸入正數美元金額，或將欄位留空。",
            "正の USD 金額を入力するか、空欄にしてください。",
        ],
        ShowDetails => ["Show Details", "显示详情", "顯示詳細資料", "詳細を表示"],
        RefreshNow => ["Refresh Now", "立即刷新", "立即重新整理", "今すぐ更新"],
        Quit => ["Quit", "退出", "結束", "終了"],
        PleaseLoginToContinue => [
            "Please login to continue",
            "请登录以继续",
            "請登入以繼續",
            "続行するにはログインしてください",
        ],
        QuotaMonitor => [
            "AI Quota Monitor",
            "AI 额度监控",
            "AI 額度監控",
            "AI クォータモニター",
        ],
        AllProviders => [
            "All providers",
            "所有服务商",
            "所有供應商",
            "すべてのプロバイダー",
        ],
        NoUsageData => [
            "No usage data",
            "无用量数据",
            "無用量資料",
            "使用量データなし",
        ],
        Error => ["Error", "错误", "錯誤", "エラー"],
        Max => ["Max", "最高", "最高", "最大"],
    }
}

pub fn refresh_age(seconds: i64) -> String {
    let language = current_language();
    if seconds < 0 {
        return language.text(Text::JustNow).to_string();
    }
    let (value, unit) = if seconds < 60 {
        (seconds, "second")
    } else {
        (seconds / 60, "minute")
    };
    match language {
        Language::English => format!("{value}{} ago", if unit == "second" { "s" } else { "m" }),
        Language::SimplifiedChinese => {
            format!("{value}{}前", if unit == "second" { "秒" } else { "分钟" })
        }
        Language::TraditionalChinese => {
            format!("{value}{}前", if unit == "second" { "秒" } else { "分鐘" })
        }
        Language::Japanese => format!("{value}{}前", if unit == "second" { "秒" } else { "分" }),
    }
}

pub fn scan_complete_message(count: usize) -> String {
    match current_language() {
        Language::English => format!(
            "Found and enabled {count} local agent{}.",
            if count == 1 { "" } else { "s" }
        ),
        Language::SimplifiedChinese => format!("已发现并启用 {count} 个本地 Agent。"),
        Language::TraditionalChinese => format!("已偵測並啟用 {count} 個本機 Agent。"),
        Language::Japanese => format!("{count} 個のローカルエージェントを検出して有効化しました。"),
    }
}

pub fn reset_count(count: i32) -> String {
    match current_language() {
        Language::English => format!("{count} Resets"),
        Language::SimplifiedChinese => format!("{count} 次重置"),
        Language::TraditionalChinese => format!("{count} 次重設"),
        Language::Japanese => format!("リセット {count} 件"),
    }
}

pub fn budget_left(amount: &str) -> String {
    match current_language() {
        Language::English => format!("Budget: {amount} USD left"),
        Language::SimplifiedChinese => format!("预算剩余：{amount} USD"),
        Language::TraditionalChinese => format!("預算剩餘：{amount} USD"),
        Language::Japanese => format!("予算残高: {amount} USD"),
    }
}

pub fn trend_count(count: usize) -> String {
    match current_language() {
        Language::English => format!(
            "7d trends · {count} window{} ",
            if count == 1 { "" } else { "s" }
        ),
        Language::SimplifiedChinese => format!("7 天趋势 · {count} 个窗口 "),
        Language::TraditionalChinese => format!("7 天趨勢 · {count} 個視窗 "),
        Language::Japanese => format!("7日間の推移 · {count} 枠 "),
    }
}

pub fn histogram_day_label(days_ago: usize) -> String {
    if days_ago == 0 {
        return match current_language() {
            Language::English => "Now",
            Language::SimplifiedChinese => "现在",
            Language::TraditionalChinese => "現在",
            Language::Japanese => "現在",
        }
        .to_string();
    }
    match current_language() {
        Language::English => format!("{days_ago}d"),
        Language::SimplifiedChinese | Language::TraditionalChinese => format!("{days_ago}天"),
        Language::Japanese => format!("{days_ago}日前"),
    }
}

pub fn trend_metrics(
    average_percent: f64,
    peak_percent: f64,
    delta: Option<f64>,
    samples: usize,
) -> String {
    let delta = delta.filter(|value| value.abs() >= 0.05).map(|value| {
        if value >= 0.0 {
            format!("+{value:.1} pp")
        } else {
            format!("{value:.1} pp")
        }
    });
    match current_language() {
        Language::English => format!(
            "avg {average_percent:.0}% · peak {peak_percent:.0}% · {} · {samples} samples",
            delta.as_deref().unwrap_or("flat")
        ),
        Language::SimplifiedChinese => format!(
            "平均 {average_percent:.0}% · 峰值 {peak_percent:.0}% · {} · {samples} 个样本",
            delta.as_deref().unwrap_or("持平")
        ),
        Language::TraditionalChinese => format!(
            "平均 {average_percent:.0}% · 峰值 {peak_percent:.0}% · {} · {samples} 個樣本",
            delta.as_deref().unwrap_or("持平")
        ),
        Language::Japanese => format!(
            "平均 {average_percent:.0}% · 最大 {peak_percent:.0}% · {} · {samples} サンプル",
            delta.as_deref().unwrap_or("横ばい")
        ),
    }
}

pub fn enabled_agents(names: &str) -> String {
    format!("{}: {names}", text(Text::Enabled))
}

pub fn new_version_available(version: &str) -> String {
    match current_language() {
        Language::English => format!("New version {version} available!"),
        Language::SimplifiedChinese => format!("新版本 {version} 已发布！"),
        Language::TraditionalChinese => format!("新版本 {version} 已發行！"),
        Language::Japanese => format!("新しいバージョン {version} が利用できます！"),
    }
}

pub fn provider_test_summary(
    windows: usize,
    max_percent: f64,
    credits: Option<(&str, &str)>,
) -> String {
    let credits = credits
        .map(|(balance, currency)| match current_language() {
            Language::English => format!(" Credits: {balance} {currency}."),
            Language::SimplifiedChinese => format!(" 余额：{balance} {currency}。"),
            Language::TraditionalChinese => format!(" 餘額：{balance} {currency}。"),
            Language::Japanese => format!(" クレジット: {balance} {currency}。"),
        })
        .unwrap_or_default();
    match current_language() {
        Language::English => {
            format!("Returned {windows} usage window(s), max usage {max_percent:.0}%.{credits}")
        }
        Language::SimplifiedChinese => {
            format!("返回 {windows} 个用量窗口，最高用量 {max_percent:.0}%。{credits}")
        }
        Language::TraditionalChinese => {
            format!("傳回 {windows} 個用量視窗，最高用量 {max_percent:.0}%。{credits}")
        }
        Language::Japanese => {
            format!("{windows} 件の使用量枠を取得、最大使用量 {max_percent:.0}%。{credits}")
        }
    }
}

pub fn paste_named(value: &str) -> String {
    match current_language() {
        Language::English => format!("Paste {value}"),
        Language::SimplifiedChinese => format!("粘贴 {value}"),
        Language::TraditionalChinese => format!("貼上 {value}"),
        Language::Japanese => format!("{value} を貼り付け"),
    }
}

pub fn login_required_for(language: Language, provider: &str, reason: &str) -> String {
    match language {
        Language::English => format!("WebView login required for {provider}: {reason}"),
        Language::SimplifiedChinese => format!("{provider} 需要 WebView 登录：{reason}"),
        Language::TraditionalChinese => format!("{provider} 需要 WebView 登入：{reason}"),
        Language::Japanese => format!("{provider} には WebView ログインが必要です: {reason}"),
    }
}

pub fn reset_duration(days: i64, hours: i64, minutes: i64) -> String {
    match current_language() {
        Language::English => {
            if days > 0 {
                format!("{days}d {hours}h")
            } else if hours > 0 {
                format!("{hours}h {minutes}m")
            } else {
                format!("{minutes}m")
            }
        }
        Language::SimplifiedChinese => {
            if days > 0 {
                format!("{days}天{hours}时")
            } else if hours > 0 {
                format!("{hours}时{minutes}分")
            } else {
                format!("{minutes}分")
            }
        }
        Language::TraditionalChinese => {
            if days > 0 {
                format!("{days}天{hours}時")
            } else if hours > 0 {
                format!("{hours}時{minutes}分")
            } else {
                format!("{minutes}分")
            }
        }
        Language::Japanese => {
            if days > 0 {
                format!("{days}日{hours}h")
            } else if hours > 0 {
                format!("{hours}h{minutes}分")
            } else {
                format!("{minutes}分")
            }
        }
    }
}

pub fn window_label(label: &str) -> Cow<'_, str> {
    window_label_for(current_language(), label)
}

pub fn window_label_for(language: Language, label: &str) -> Cow<'_, str> {
    let translated = match label.trim() {
        "Session" => ["Session", "会话", "工作階段", "セッション"],
        "Session (5h)" => [
            "Session (5h)",
            "会话（5 小时）",
            "工作階段（5 小時）",
            "セッション（5時間）",
        ],
        "Rolling Usage" => ["Rolling Usage", "滚动用量", "滾動用量", "ローリング使用量"],
        "Weekly" | "Weekly Usage" => ["Weekly Usage", "周用量", "週用量", "週間使用量"],
        "Monthly" | "Monthly Usage" => ["Monthly Usage", "月用量", "月用量", "月間使用量"],
        "Today Spend" => ["Today Spend", "今日费用", "今日費用", "今日の支出"],
        "Week Spend" | "7d Spend" => ["Week Spend", "本周费用", "本週費用", "今週の支出"],
        "Month Spend" | "30d Spend" | "Cost 30d" => {
            ["Month Spend", "本月费用", "本月費用", "今月の支出"]
        }
        "Last Full Day Spend" => [
            "Last Full Day Spend",
            "上一完整日费用",
            "上一完整日費用",
            "前日の支出",
        ],
        "Credits" => ["Credits", "余额", "餘額", "クレジット"],
        "Balance" => ["Balance", "余额", "餘額", "残高"],
        "Usage" => ["Usage", "用量", "用量", "使用量"],
        "Quota" => ["Quota", "额度", "額度", "クォータ"],
        "Requests" => ["Requests", "请求", "請求", "リクエスト"],
        "Characters" => ["Characters", "字符", "字元", "文字数"],
        "Input Tokens" => ["Input Tokens", "输入 Token", "輸入 Token", "入力トークン"],
        "Output Tokens" => ["Output Tokens", "输出 Token", "輸出 Token", "出力トークン"],
        "Billing Cycle" => ["Billing Cycle", "计费周期", "計費週期", "請求サイクル"],
        "On-Demand" => ["On-Demand", "按需计费", "隨用隨付", "従量課金"],
        "Subscription" => ["Subscription", "订阅", "訂閱", "サブスクリプション"],
        "Connected" => ["Connected", "已连接", "已連線", "接続済み"],
        "No data" => ["No data", "无数据", "無資料", "データなし"],
        "Error" => ["Error", "错误", "錯誤", "エラー"],
        _ => return Cow::Borrowed(label),
    };
    Cow::Borrowed(translated[language.index()])
}

pub fn credit_status(status: &str) -> Cow<'_, str> {
    let translated = match status.trim().to_ascii_lowercase().as_str() {
        "available" => ["Available", "可用", "可用", "利用可能"],
        "used" => ["Used", "已使用", "已使用", "使用済み"],
        "expired" => ["Expired", "已过期", "已過期", "期限切れ"],
        "unavailable" => ["Unavailable", "不可用", "不可用", "利用不可"],
        "unknown" | "" => ["Unknown", "未知", "未知", "不明"],
        _ => return Cow::Borrowed(status),
    };
    Cow::Borrowed(translated[current_language().index()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_windows_locales() {
        assert_eq!(language_from_locale("zh-CN"), Language::SimplifiedChinese);
        assert_eq!(
            language_from_locale("zh-Hans-SG"),
            Language::SimplifiedChinese
        );
        assert_eq!(language_from_locale("zh-TW"), Language::TraditionalChinese);
        assert_eq!(
            language_from_locale("zh-Hant-HK"),
            Language::TraditionalChinese
        );
        assert_eq!(language_from_locale("ja-JP"), Language::Japanese);
        assert_eq!(language_from_locale("en-US"), Language::English);
    }

    #[test]
    fn config_values_round_trip() {
        for setting in LanguageSetting::ALL {
            assert_eq!(
                LanguageSetting::from_config(setting.config_value()),
                setting
            );
            assert!(!setting.display_name().is_empty());
        }
    }

    #[test]
    fn maps_languages_to_gpui_component_locales() {
        assert_eq!(Language::English.component_locale(), "en");
        assert_eq!(Language::SimplifiedChinese.component_locale(), "zh-CN");
        assert_eq!(Language::TraditionalChinese.component_locale(), "zh-HK");
        assert_eq!(Language::Japanese.component_locale(), "ja");
    }

    #[test]
    fn all_supported_languages_have_core_translations() {
        for language in [
            Language::English,
            Language::SimplifiedChinese,
            Language::TraditionalChinese,
            Language::Japanese,
        ] {
            assert!(!language.text(Text::Settings).is_empty());
            assert!(!language.text(Text::Notifications).is_empty());
            assert!(!language.text(Text::ProviderSettings).is_empty());
        }
    }
}
