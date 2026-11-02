//! Redmine REST API 客户端。需管理员在“管理 → 配置 → API”中启用 REST web service。

use crate::config::RedmineConfig;
use crate::models::{Assignee, Issue, NamedId, Project, ProjectAccess, User};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone)]
pub struct RedmineClient {
    http: reqwest::Client,
    base: String,
    key: String,
}

/// 列表查询参数，对应 `/issues.json` 的过滤条件
#[derive(Debug, Clone, Default)]
pub struct IssueQuery {
    pub assigned_to_me: bool,
    pub watched_by_me: bool,
    pub project: Option<String>,
    pub query_id: Option<u64>,
    /// open | closed | * | 具体状态 ID
    pub status: String,
    pub updated_since: Option<String>,
    /// 其他筛选参数，如 `closed_on`
    pub extra: Vec<(&'static str, String)>,
    /// 最多拉取多少条，0 表示默认上限 2000
    pub max_results: usize,
}

/// Redmine 版本（里程碑）
#[derive(Debug, Clone, Deserialize)]
pub struct Version {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub due_date: Option<String>,
}

/// 任务类别，可设置默认负责人
#[derive(Debug, Clone, Deserialize)]
pub struct IssueCategory {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub assigned_to: Option<NamedId>,
}

#[derive(Debug, Default)]
pub struct IssueUpdate {
    pub notes: Option<String>,
    pub private_notes: bool,
    pub status_id: Option<u64>,
    pub assigned_to_id: Option<u64>,
    pub done_ratio: Option<u32>,
    pub uploads: Vec<UploadRef>,
}

/// `/uploads.json` 返回的令牌，随任务更新一起提交后附件才会挂到任务上（令牌只能用一次）
#[derive(Debug, Clone)]
pub struct UploadRef {
    pub token: String,
    pub filename: String,
    pub content_type: Option<String>,
}

impl RedmineClient {
    pub fn new(cfg: &RedmineConfig, api_key: &str) -> Result<Self> {
        let base = cfg.base_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            bail!("尚未配置 Redmine 地址");
        }
        if api_key.trim().is_empty() {
            bail!("尚未配置 Redmine API Key（在 Redmine“我的账号”页面右侧可查看）");
        }
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("RedPilot/", env!("CARGO_PKG_VERSION")))
            .danger_accept_invalid_certs(cfg.accept_invalid_certs);
        builder = apply_proxy(builder, &cfg.proxy, &base)?;
        Ok(Self {
            http: builder.build()?,
            base,
            key: api_key.trim().to_string(),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn issue_url(&self, id: u64) -> String {
        format!("{}/issues/{}", self.base, id)
    }

    async fn request(&self, method: Method, path: &str, query: &[(&str, String)], body: Option<Value>) -> Result<Option<Value>> {
        let url = format!("{}{}", self.base, path);
        let mut req = self
            .http
            .request(method, &url)
            .header("X-Redmine-API-Key", &self.key)
            .query(query);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| anyhow!("连接 Redmine 失败: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            if text.trim().is_empty() {
                return Ok(None);
            }
            return Ok(Some(serde_json::from_str(&text).context("Redmine 返回了无法解析的内容")?));
        }
        Err(map_error(status, &text))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let v = self
            .request(Method::GET, path, query, None)
            .await?
            .ok_or_else(|| anyhow!("Redmine 返回空响应: {path}"))?;
        Ok(serde_json::from_value(v)?)
    }

    pub async fn current_user(&self) -> Result<User> {
        #[derive(Deserialize)]
        struct W {
            user: User,
        }
        Ok(self.get::<W>("/users/current.json", &[]).await?.user)
    }

    /// 分页拉取全部匹配任务（单次最多 100 条）
    pub async fn list_issues(&self, q: &IssueQuery) -> Result<Vec<Issue>> {
        #[derive(Deserialize)]
        struct Page {
            issues: Vec<Issue>,
            #[serde(default)]
            total_count: usize,
        }
        let mut params: Vec<(&str, String)> = vec![
            ("status_id", if q.status.is_empty() { "open".into() } else { q.status.clone() }),
            ("sort", "updated_on:desc".into()),
            ("limit", "100".into()),
        ];
        if q.assigned_to_me {
            params.push(("assigned_to_id", "me".into()));
        }
        if q.watched_by_me {
            params.push(("watcher_id", "me".into()));
        }
        if let Some(p) = q.project.as_ref().filter(|p| !p.trim().is_empty()) {
            params.push(("project_id", p.trim().to_string()));
        }
        if let Some(id) = q.query_id {
            params.push(("query_id", id.to_string()));
        }
        if let Some(since) = &q.updated_since {
            params.push(("updated_on", format!(">={since}")));
        }
        params.extend(q.extra.iter().cloned());
        let cap = if q.max_results == 0 { 2000 } else { q.max_results.min(2000) };
        let mut all = Vec::new();
        let mut offset = 0usize;
        loop {
            let mut p = params.clone();
            p.push(("offset", offset.to_string()));
            let page: Page = self.get("/issues.json", &p).await?;
            let n = page.issues.len();
            all.extend(page.issues);
            offset += n;
            // 上限保护，避免误配查询拉取整个库
            if n == 0 || offset >= page.total_count || offset >= cap {
                break;
            }
        }
        all.truncate(cap);
        Ok(all)
    }

    /// 只取符合条件的任务数量
    pub async fn count_issues(&self, params: &[(&str, String)]) -> Result<usize> {
        #[derive(Deserialize)]
        struct Page {
            #[serde(default)]
            total_count: usize,
        }
        let mut p: Vec<(&str, String)> = params.to_vec();
        p.push(("limit", "1".into()));
        Ok(self.get::<Page>("/issues.json", &p).await?.total_count)
    }

    /// 我参与的项目和角色；系统管理员可以看到全部项目
    pub async fn my_projects(&self) -> Result<(bool, Vec<ProjectAccess>)> {
        #[derive(Deserialize)]
        struct M {
            project: NamedId,
            #[serde(default)]
            roles: Vec<NamedId>,
        }
        #[derive(Deserialize)]
        struct U {
            #[serde(default)]
            admin: bool,
            #[serde(default)]
            memberships: Vec<M>,
        }
        #[derive(Deserialize)]
        struct W {
            user: U,
        }
        let u = self
            .get::<W>("/users/current.json", &[("include", "memberships".into())])
            .await?
            .user;
        let mut out: Vec<ProjectAccess> = Vec::new();
        for m in u.memberships {
            let roles: Vec<String> = m.roles.into_iter().map(|r| r.name).collect();
            match out.iter_mut().find(|p| p.id == m.project.id) {
                Some(p) => {
                    for r in roles {
                        if !p.roles.contains(&r) {
                            p.roles.push(r);
                        }
                    }
                }
                None => out.push(ProjectAccess { id: m.project.id, name: m.project.name, roles, is_manager: false }),
            }
        }
        Ok((u.admin, out))
    }

    pub async fn versions(&self, project_id: u64) -> Result<Vec<Version>> {
        #[derive(Deserialize)]
        struct W {
            versions: Vec<Version>,
        }
        Ok(self.get::<W>(&format!("/projects/{project_id}/versions.json"), &[]).await?.versions)
    }

    pub async fn issue_categories(&self, project_id: u64) -> Result<Vec<IssueCategory>> {
        #[derive(Deserialize)]
        struct W {
            issue_categories: Vec<IssueCategory>,
        }
        Ok(self
            .get::<W>(&format!("/projects/{project_id}/issue_categories.json"), &[])
            .await?
            .issue_categories)
    }

    pub async fn get_issue(&self, id: u64) -> Result<Issue> {
        #[derive(Deserialize)]
        struct W {
            issue: Issue,
        }
        let include = "journals,attachments,relations,watchers,allowed_statuses";
        Ok(self
            .get::<W>(&format!("/issues/{id}.json"), &[("include", include.into())])
            .await?
            .issue)
    }

    pub async fn update_issue(&self, id: u64, u: &IssueUpdate) -> Result<()> {
        let mut issue = serde_json::Map::new();
        if let Some(n) = &u.notes {
            issue.insert("notes".into(), json!(n));
            if u.private_notes {
                issue.insert("private_notes".into(), json!(true));
            }
        }
        if let Some(s) = u.status_id {
            issue.insert("status_id".into(), json!(s));
        }
        if let Some(a) = u.assigned_to_id {
            issue.insert("assigned_to_id".into(), json!(a));
        }
        if let Some(d) = u.done_ratio {
            issue.insert("done_ratio".into(), json!(d));
        }
        if !u.uploads.is_empty() {
            let ups: Vec<Value> = u
                .uploads
                .iter()
                .map(|x| {
                    let mut o = json!({ "token": x.token, "filename": x.filename });
                    if let Some(ct) = &x.content_type {
                        o["content_type"] = json!(ct);
                    }
                    o
                })
                .collect();
            issue.insert("uploads".into(), json!(ups));
        }
        self.request(Method::PUT, &format!("/issues/{id}.json"), &[], Some(json!({ "issue": issue })))
            .await?;
        Ok(())
    }

    /// 下载附件内容（附件需要 API Key 才能访问，界面无法直接引用 Redmine 上的地址）
    pub async fn download_attachment(&self, id: u64, max_bytes: u64) -> Result<Vec<u8>> {
        let resp = self
            .http
            .get(format!("{}/attachments/download/{id}", self.base))
            .header("X-Redmine-API-Key", &self.key)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| anyhow!("下载附件失败: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            if status == StatusCode::NOT_FOUND {
                return Err(crate::coded("REDMINE_NOT_FOUND", "附件不存在或无权查看"));
            }
            let text = resp.text().await.unwrap_or_default();
            return Err(map_error(status, &text));
        }
        if resp.content_length().is_some_and(|n| n > max_bytes) {
            return Err(crate::coded("ATTACHMENT_TOO_LARGE", "附件太大，无法预览，请在浏览器中打开"));
        }
        let bytes = resp.bytes().await.map_err(|e| anyhow!("下载附件失败: {e}"))?;
        if bytes.len() as u64 > max_bytes {
            return Err(crate::coded("ATTACHMENT_TOO_LARGE", "附件太大，无法预览，请在浏览器中打开"));
        }
        Ok(bytes.to_vec())
    }

    /// 上传文件，返回附件令牌
    pub async fn upload(&self, bytes: Vec<u8>, filename: &str, content_type: Option<&str>) -> Result<UploadRef> {
        #[derive(Deserialize)]
        struct Up {
            token: String,
        }
        #[derive(Deserialize)]
        struct W {
            upload: Up,
        }
        let resp = self
            .http
            .post(format!("{}/uploads.json", self.base))
            .header("X-Redmine-API-Key", &self.key)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .query(&[("filename", filename)])
            .timeout(Duration::from_secs(300))
            .body(bytes)
            .send()
            .await
            .map_err(|e| anyhow!("上传附件失败: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            if status == StatusCode::PAYLOAD_TOO_LARGE || text.contains("size") || text.contains("大小") {
                return Err(crate::coded("UPLOAD_TOO_LARGE", format!("附件 {filename} 超过 Redmine 允许的大小（管理 → 配置 → 文件 中可调整）")));
            }
            return Err(map_error(status, &text));
        }
        let w: W = serde_json::from_str(&text).context("Redmine 上传接口返回了无法解析的内容")?;
        Ok(UploadRef {
            token: w.upload.token,
            filename: filename.to_string(),
            content_type: content_type.map(str::to_string),
        })
    }

    pub async fn create_time_entry(&self, issue_id: u64, hours: f64, activity_id: Option<u64>, comments: &str) -> Result<()> {
        let mut entry = json!({ "issue_id": issue_id, "hours": hours, "comments": truncate(comments, 250) });
        if let Some(a) = activity_id {
            entry["activity_id"] = json!(a);
        }
        self.request(Method::POST, "/time_entries.json", &[], Some(json!({ "time_entry": entry })))
            .await?;
        Ok(())
    }

    pub async fn create_relation(&self, issue_id: u64, issue_to_id: u64, relation_type: &str) -> Result<()> {
        self.request(
            Method::POST,
            &format!("/issues/{issue_id}/relations.json"),
            &[],
            Some(json!({ "relation": { "issue_to_id": issue_to_id, "relation_type": relation_type } })),
        )
        .await?;
        Ok(())
    }

    /// 项目成员（用户和组），作为指派候选
    pub async fn memberships(&self, project_id: u64) -> Result<Vec<Assignee>> {
        #[derive(Deserialize)]
        struct M {
            #[serde(default)]
            user: Option<NamedId>,
            #[serde(default)]
            group: Option<NamedId>,
            #[serde(default)]
            roles: Vec<NamedId>,
        }
        #[derive(Deserialize)]
        struct Page {
            memberships: Vec<M>,
            #[serde(default)]
            total_count: usize,
        }
        let mut out: Vec<Assignee> = Vec::new();
        let mut offset = 0usize;
        loop {
            let page: Page = self
                .get(
                    &format!("/projects/{project_id}/memberships.json"),
                    &[("limit", "100".into()), ("offset", offset.to_string())],
                )
                .await?;
            let n = page.memberships.len();
            for m in page.memberships {
                let (who, is_group) = match (m.user, m.group) {
                    (Some(u), _) => (u, false),
                    (None, Some(g)) => (g, true),
                    _ => continue,
                };
                let roles: Vec<String> = m.roles.into_iter().map(|r| r.name).collect();
                // 通过组继承角色的用户会重复出现，合并角色
                match out.iter_mut().find(|a| a.id == who.id) {
                    Some(a) => {
                        for r in roles {
                            if !a.roles.contains(&r) {
                                a.roles.push(r);
                            }
                        }
                    }
                    None => out.push(Assignee { id: who.id, name: who.name, is_group, roles }),
                }
            }
            offset += n;
            if n == 0 || offset >= page.total_count || offset >= 2000 {
                break;
            }
        }
        Ok(out)
    }

    pub async fn statuses(&self) -> Result<Vec<NamedId>> {
        #[derive(Deserialize)]
        struct W {
            issue_statuses: Vec<NamedId>,
        }
        Ok(self.get::<W>("/issue_statuses.json", &[]).await?.issue_statuses)
    }

    pub async fn activities(&self) -> Result<Vec<NamedId>> {
        #[derive(Deserialize)]
        struct W {
            time_entry_activities: Vec<NamedId>,
        }
        Ok(self
            .get::<W>("/enumerations/time_entry_activities.json", &[])
            .await?
            .time_entry_activities)
    }

    pub async fn projects(&self) -> Result<Vec<Project>> {
        #[derive(Deserialize)]
        struct W {
            projects: Vec<Project>,
        }
        Ok(self.get::<W>("/projects.json", &[("limit", "100".into())]).await?.projects)
    }
}

fn map_error(status: StatusCode, body: &str) -> anyhow::Error {
    #[derive(Deserialize)]
    struct Errs {
        errors: Vec<String>,
    }
    let detail = serde_json::from_str::<Errs>(body)
        .map(|e| e.errors.join("；"))
        .unwrap_or_else(|_| truncate(body.trim(), 200));
    match status {
        StatusCode::UNAUTHORIZED => crate::coded("REDMINE_AUTH", "API Key 无效，或服务器未启用 REST API"),
        StatusCode::FORBIDDEN => crate::coded("REDMINE_FORBIDDEN", format!("没有权限执行该操作 {detail}")),
        StatusCode::NOT_FOUND => crate::coded("REDMINE_NOT_FOUND", "任务不存在或无权查看"),
        StatusCode::UNPROCESSABLE_ENTITY => {
            let lower = detail.to_lowercase();
            if lower.contains("assignee") || detail.contains("指派") {
                crate::coded("ASSIGNEE_INVALID", format!("无法指派给目标用户（可能不是项目成员，或其角色不允许被指派）：{detail}"))
            } else if lower.contains("status") || detail.contains("状态") {
                crate::coded("STATUS_INVALID", format!("工作流不允许切换到该状态：{detail}"))
            } else {
                crate::coded("REDMINE_VALIDATION", detail)
            }
        }
        _ => crate::coded("REDMINE_HTTP", format!("HTTP {status}: {detail}")),
    }
}

/// 代理策略：`direct` 强制直连；填写地址则走该代理；留空使用系统代理，
/// 但本机与内网地址（127/8、10/8、172.16/12、192.168/16、*.local、无点主机名）始终直连，
/// 避免系统代理把内网 Redmine 请求转发到外网。
pub fn apply_proxy(builder: reqwest::ClientBuilder, proxy: &str, target_url: &str) -> Result<reqwest::ClientBuilder> {
    let proxy = proxy.trim();
    if proxy.eq_ignore_ascii_case("direct") || (proxy.is_empty() && is_local_target(target_url)) {
        return Ok(builder.no_proxy());
    }
    if proxy.is_empty() {
        return Ok(builder);
    }
    Ok(builder.proxy(reqwest::Proxy::all(proxy).context("代理地址格式错误")?))
}

fn is_local_target(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let host = if host.starts_with('[') {
        host.trim_start_matches('[').split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") || (!host.is_empty() && !host.contains('.') && !host.contains(':')) {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}

pub fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn downloads_attachments_with_api_key() {
        use axum::extract::Path as AxPath;
        use axum::http::HeaderMap;
        let app = axum::Router::new().route(
            "/attachments/download/{id}",
            axum::routing::get(|AxPath(id): AxPath<u64>, h: HeaderMap| async move {
                if h.get("X-Redmine-API-Key").and_then(|v| v.to_str().ok()) != Some("k") {
                    return (StatusCode::UNAUTHORIZED, Vec::new());
                }
                match id {
                    7 => (StatusCode::OK, b"\x89PNG-data".to_vec()),
                    8 => (StatusCode::OK, vec![0u8; 2048]),
                    _ => (StatusCode::NOT_FOUND, Vec::new()),
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let cfg = RedmineConfig { base_url: format!("http://{addr}"), proxy: "direct".into(), ..Default::default() };

        let rm = RedmineClient::new(&cfg, "k").unwrap();
        assert_eq!(rm.download_attachment(7, 1024).await.unwrap(), b"\x89PNG-data");
        assert!(rm.download_attachment(9, 1024).await.unwrap_err().to_string().contains("REDMINE_NOT_FOUND"));
        assert!(rm.download_attachment(8, 1024).await.unwrap_err().to_string().contains("ATTACHMENT_TOO_LARGE"));
        let bad = RedmineClient::new(&cfg, "wrong").unwrap();
        assert!(bad.download_attachment(7, 1024).await.unwrap_err().to_string().contains("REDMINE_AUTH"));
    }

    #[test]
    fn detects_intranet_targets() {
        assert!(is_local_target("http://127.0.0.1:8080/x"));
        assert!(is_local_target("https://192.168.1.10"));
        assert!(is_local_target("http://10.2.3.4:3000"));
        assert!(is_local_target("http://redmine/issues"));
        assert!(is_local_target("http://localhost:11434/v1"));
        assert!(is_local_target("http://[::1]:8080"));
        assert!(!is_local_target("https://redmine.example.com"));
        assert!(!is_local_target("https://api.deepseek.com"));
    }
}
