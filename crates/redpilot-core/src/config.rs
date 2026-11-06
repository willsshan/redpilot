use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 数据目录：配置、SQLite 缓存、（钥匙串不可用时的）密钥文件都放在这里。
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    /// 默认位置：macOS `~/Library/Application Support/RedPilot`，
    /// Windows `%APPDATA%\RedPilot`。可用环境变量 `REDPILOT_HOME` 覆盖。
    pub fn default_location() -> Result<Self> {
        let root = match std::env::var_os("REDPILOT_HOME") {
            Some(p) => PathBuf::from(p),
            None => dirs::data_dir()
                .context("无法定位系统数据目录")?
                .join("RedPilot"),
        };
        std::fs::create_dir_all(&root).with_context(|| format!("创建数据目录失败: {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub fn db_file(&self) -> PathBuf {
        self.root.join("redpilot.db")
    }

    pub fn secrets_file(&self) -> PathBuf {
        self.root.join("secrets.json")
    }

    /// 回派前暂存的附件
    pub fn staging_dir(&self) -> PathBuf {
        self.root.join("staging")
    }

    /// 已下载的附件（用于在界面中预览图片）
    pub fn attachment_cache_dir(&self) -> PathBuf {
        self.root.join("cache").join("attachments")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppConfig {
    pub redmine: RedmineConfig,
    pub sync: SyncConfig,
    pub workflow: WorkflowConfig,
    pub ai: AiConfig,
    pub mcp: McpConfig,
    pub rules: Vec<CategoryRule>,
    pub dispatch: DispatchConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            redmine: RedmineConfig::default(),
            sync: SyncConfig::default(),
            workflow: WorkflowConfig::default(),
            ai: AiConfig::default(),
            mcp: McpConfig::default(),
            rules: default_rules(),
            dispatch: DispatchConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextFormat {
    Markdown,
    Textile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RedmineConfig {
    /// 例如 `https://redmine.example.com`
    pub base_url: String,
    /// 内网自签名证书时可开启（会有安全风险提示）
    pub accept_invalid_certs: bool,
    /// HTTP/HTTPS 代理，例如 `http://127.0.0.1:7890`；留空使用系统代理（内网地址自动直连）；`direct` 强制直连
    pub proxy: String,
    /// 服务器“文本格式”设置，决定处置说明使用的标记语法
    pub text_format: TextFormat,
}

impl Default for RedmineConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            accept_invalid_certs: false,
            proxy: String::new(),
            text_format: TextFormat::Markdown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SyncConfig {
    /// 自动同步间隔（分钟），0 表示关闭自动同步
    pub interval_minutes: u32,
    pub assigned_to_me: bool,
    pub watched: bool,
    /// 限定项目（标识符或 ID），留空为全部项目
    pub project: String,
    /// 额外拉取某个 Redmine 已保存查询
    pub query_id: Option<u64>,
    /// 有新任务时发系统通知
    pub notify: bool,
    /// 提出人回复、任务被转回、调为高优先级时发通知
    pub notify_changes: bool,
    /// 任务今天或明天到期时提醒（每天一次）
    pub notify_due: bool,
    /// 新任务自动做 AI 分析
    pub auto_analyze: bool,
    /// 同时拉取我管理的项目里未指派的任务（进入分派台）
    pub managed_unassigned: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            interval_minutes: 5,
            assigned_to_me: true,
            watched: false,
            project: String::new(),
            query_id: None,
            notify: true,
            notify_changes: true,
            notify_due: true,
            auto_analyze: true,
            managed_unassigned: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowConfig {
    /// “处理完成”后要切换到的状态（如：已解决 / 待验证）
    pub resolved_status_id: Option<u64>,
    /// “开始处理”时切换到的状态（如：进行中）
    pub in_progress_status_id: Option<u64>,
    /// 追问提出人时切换到的状态（Redmine 默认的“反馈”）
    pub feedback_status_id: Option<u64>,
    /// 处理完成时把完成度设为 100%
    pub set_done_ratio: bool,
    /// 登记工时默认活动类型
    pub default_activity_id: Option<u64>,
    /// 处置说明模板中的小节标题
    pub note_sections: Vec<String>,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            resolved_status_id: None,
            in_progress_status_id: None,
            feedback_status_id: None,
            set_done_ratio: true,
            default_activity_id: None,
            note_sections: vec![
                "问题原因".into(),
                "处理方法".into(),
                "处理结果".into(),
                "影响范围".into(),
                "验证建议".into(),
            ],
        }
    }
}

/// 分派台：开发经理把收到的问题派给团队成员
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DispatchConfig {
    /// 指派给我、处于这些状态的任务进入分派台；留空时自动识别“新建”一类的状态
    pub pending_status_ids: Vec<u64>,
    /// 拉取未指派任务的项目；留空为我担任管理角色的项目
    pub project_ids: Vec<u64>,
    /// 派单时把我加为关注人，便于跟进
    pub watch: bool,
    /// 任务没有截止日期时，按优先级给的默认期限（工作日）
    pub due_days: DueDays,
    /// 派单后切换到的状态，留空不改
    pub assigned_status_id: Option<u64>,
    /// 派单备注模板，可用 {负责人} {提出人} {截止日期}
    pub note_template: String,
    /// 团队分工与常用成员，顺序就是分派台里数字键的顺序
    pub team: Vec<TeamMember>,
}

impl Default for DispatchConfig {
    fn default() -> Self {
        Self {
            pending_status_ids: vec![],
            project_ids: vec![],
            watch: true,
            due_days: DueDays::default(),
            assigned_status_id: None,
            note_template: String::new(),
            team: vec![],
        }
    }
}

/// 各优先级的默认期限（工作日），0 表示不设置
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DueDays {
    pub urgent: u32,
    pub high: u32,
    pub normal: u32,
    pub low: u32,
}

impl Default for DueDays {
    fn default() -> Self {
        Self { urgent: 1, high: 3, normal: 5, low: 0 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TeamMember {
    pub id: u64,
    pub name: String,
    /// 分工方向：前端、后端、全栈、测试、运维、数据、产品，留空表示未设置
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiConfig {
    pub enabled: bool,
    /// OpenAI 兼容接口地址：DeepSeek 官方、私有化部署、Ollama、vLLM 均可
    pub base_url: String,
    /// 日常分析使用的模型
    pub model: String,
    /// 合并规划使用的模型，留空则与 `model` 相同（可填推理模型）
    pub planning_model: String,
    /// 请求 JSON 输出模式（部分推理模型不支持时关闭）
    pub json_mode: bool,
    pub temperature: f32,
    /// 发送前对手机号、身份证、IP、密码等脱敏
    pub mask_sensitive: bool,
    /// 这些项目的内容不发送给 AI（项目 ID）
    pub excluded_project_ids: Vec<u64>,
    /// 每次请求合并分析的任务数
    pub batch_size: usize,
    /// 每百万输入 token 单价（用于估算费用）
    pub price_input: f64,
    /// 每百万输出 token 单价
    pub price_output: f64,
    pub currency: String,
    /// 追加到各环节系统提示词后面的团队要求
    pub custom_prompts: CustomPrompts,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CustomPrompts {
    pub classify: String,
    pub plan: String,
    pub note: String,
    pub report: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-chat".into(),
            planning_model: String::new(),
            json_mode: true,
            temperature: 0.2,
            mask_sensitive: true,
            excluded_project_ids: vec![],
            batch_size: 8,
            // DeepSeek 官网标价（缓存未命中），价格调整时在设置中修改
            price_input: 2.0,
            price_output: 3.0,
            currency: "¥".into(),
            custom_prompts: CustomPrompts::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct McpConfig {
    /// 在本机启动 Streamable HTTP MCP 服务
    pub enabled: bool,
    pub port: u16,
    /// HTTP 访问令牌（Authorization: Bearer <token>）
    pub token: String,
    /// 是否允许外部 AI 执行写操作（回派、回复、改状态、建关联）
    pub allow_write: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 39217,
            token: String::new(),
            allow_write: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryRule {
    pub category: String,
    pub keywords: Vec<String>,
}

pub fn default_rules() -> Vec<CategoryRule> {
    let r = |c: &str, k: &[&str]| CategoryRule {
        category: c.into(),
        keywords: k.iter().map(|s| s.to_string()).collect(),
    };
    vec![
        r("缺陷", &["报错", "错误", "异常", "bug", "崩溃", "失败", "无法", "不能", "闪退", "白屏", "500", "exception"]),
        r("权限配置", &["权限", "账号", "密码", "开通", "角色", "授权", "登录不了", "配置"]),
        r("数据处理", &["数据", "导入", "导出", "修改数据", "删除数据", "统计", "报表", "sql"]),
        r("环境问题", &["服务器", "部署", "网络", "超时", "磁盘", "内存", "宕机", "证书", "环境"]),
        r("需求", &["新增", "增加", "希望", "需求", "优化", "建议", "改进", "支持"]),
        r("咨询", &["如何", "怎么", "请问", "是否", "能否", "咨询"]),
    ]
}

/// 分类候选：规则里出现的分类 + “其他”
pub fn categories(cfg: &AppConfig) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in &cfg.rules {
        if !out.contains(&r.category) {
            out.push(r.category.clone());
        }
    }
    if !out.iter().any(|c| c == "其他") {
        out.push("其他".into());
    }
    out
}

pub fn load(path: &Path) -> Result<AppConfig> {
    let mut cfg: AppConfig = if path.exists() {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).with_context(|| format!("配置文件格式错误: {}", path.display()))?
    } else {
        AppConfig::default()
    };
    if cfg.mcp.token.is_empty() {
        cfg.mcp.token = new_token();
        save(path, &cfg)?;
    }
    Ok(cfg)
}

pub fn save(path: &Path, cfg: &AppConfig) -> Result<()> {
    let text = serde_json::to_string_pretty(cfg)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn new_token() -> String {
    format!("rp_{}", uuid::Uuid::new_v4().simple())
}
