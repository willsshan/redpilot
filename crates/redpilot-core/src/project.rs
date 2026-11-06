//! 项目总览：项目视角的任务、进度统计，以及转派时的负责人推荐（本地算法，不消耗 token）。

use crate::analysis::{cosine, Corpus};
use crate::config::DueDays;
use crate::models::{AssignSuggestion, Assignee, Issue, MultiPlan, NamedId, ProjectIssue, SplitPart};
use crate::redmine::{truncate, IssueCategory};
use chrono::{Datelike, NaiveDate};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// 角色名里带这些词的视为项目管理员
const MANAGER_WORDS: [&str; 9] = ["管理", "经理", "负责人", "主管", "manager", "lead", "owner", "admin", "pm"];
/// 只读类角色，推荐负责人时跳过
const READONLY_WORDS: [&str; 6] = ["报告", "观察", "查看", "reporter", "viewer", "guest"];

pub fn is_manager_role(name: &str) -> bool {
    let n = name.to_lowercase();
    MANAGER_WORDS.iter().any(|w| n.contains(w))
}

fn is_readonly_role(name: &str) -> bool {
    let n = name.to_lowercase();
    READONLY_WORDS.iter().any(|w| n.contains(w))
}

pub fn to_project_issue(is: &Issue, today: NaiveDate) -> ProjectIssue {
    let due = is.due_date.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let updated = chrono::DateTime::parse_from_rfc3339(&is.updated_on).ok().map(|t| t.date_naive());
    ProjectIssue {
        id: is.id,
        project_name: is.project.name.clone(),
        subject: is.subject.clone(),
        description: truncate(is.description.as_deref().unwrap_or("").trim(), 400),
        tracker: is.tracker.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
        status: is.status.name.clone(),
        priority_id: is.priority.as_ref().map(|p| p.id).unwrap_or(0),
        priority: is.priority.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
        author_id: is.author.as_ref().map(|a| a.id),
        author: is.author.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
        assignee_id: is.assigned_to.as_ref().map(|a| a.id),
        assignee: is.assigned_to.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
        category_id: is.category.as_ref().map(|c| c.id),
        category: is.category.as_ref().map(|c| c.name.clone()),
        version_id: is.fixed_version.as_ref().map(|v| v.id),
        version: is.fixed_version.as_ref().map(|v| v.name.clone()),
        done_ratio: is.done_ratio.unwrap_or(0),
        due_date: is.due_date.clone(),
        created_on: is.created_on.clone(),
        updated_on: is.updated_on.clone(),
        parent_id: is.parent.as_ref().map(|p| p.id),
        overdue: due.is_some_and(|d| d < today),
        stale_days: updated.map(|u| (today - u).num_days().max(0)).unwrap_or(0),
    }
}

/// 最近 n 周（含本周）每周的周一和周日，从早到晚
pub fn recent_weeks(today: NaiveDate, n: usize) -> Vec<(NaiveDate, NaiveDate)> {
    let monday = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    (0..n)
        .rev()
        .map(|i| {
            let start = monday - chrono::Duration::weeks(i as i64);
            (start, start + chrono::Duration::days(6))
        })
        .collect()
}

/// 可以作为推荐对象的成员：用户（不含组），且不是只读角色
pub fn candidates(members: &[Assignee]) -> Vec<&Assignee> {
    members
        .iter()
        .filter(|m| !m.is_group && !(m.roles.iter().all(|r| is_readonly_role(r)) && !m.roles.is_empty()))
        .collect()
}

fn doc(i: &ProjectIssue) -> String {
    format!(
        "{s} {s} {} {} {}",
        i.category.as_deref().unwrap_or(""),
        i.tracker,
        truncate(&i.description, 200),
        s = i.subject
    )
}

/// 为每个待转派任务推荐负责人：
/// 类别默认负责人 > 相似任务现在由谁负责 > 负载最低的成员。负载越高得分越低。
///
/// - `refs`：用来判断“谁熟悉什么”的参考任务（其他未关闭任务，以及近期关闭且没有回派给提出人的任务）
/// - `load`：每个成员当前负责的未关闭任务数
pub fn local_suggestions(
    targets: &[ProjectIssue],
    refs: &[ProjectIssue],
    members: &[Assignee],
    categories: &[IssueCategory],
    load: &HashMap<u64, usize>,
) -> Vec<AssignSuggestion> {
    suggest_with_roles(targets, refs, members, categories, load, &HashMap::new())
}

/// 同 [`local_suggestions`]，另外按成员的分工方向加分：任务涉及前端时优先前端成员
pub fn suggest_with_roles(
    targets: &[ProjectIssue],
    refs: &[ProjectIssue],
    members: &[Assignee],
    categories: &[IssueCategory],
    load: &HashMap<u64, usize>,
    roles: &HashMap<u64, String>,
) -> Vec<AssignSuggestion> {
    let cands = candidates(members);
    let name_of = |id: u64| members.iter().find(|m| m.id == id).map(|m| m.name.clone());
    let is_cand = |id: u64| cands.iter().any(|c| c.id == id);
    // 参考任务只保留有负责人、且负责人是候选成员的
    let refs: Vec<&ProjectIssue> = refs
        .iter()
        .filter(|r| r.assignee_id.is_some_and(is_cand) && !targets.iter().any(|t| t.id == r.id))
        .collect();
    let mut docs: Vec<String> = refs.iter().map(|r| doc(r)).collect();
    docs.extend(targets.iter().map(doc));
    let corpus = Corpus::build(&docs);

    targets
        .iter()
        .enumerate()
        .map(|(ti, t)| {
            let tv = &corpus.vectors[refs.len() + ti];
            let mut score: HashMap<u64, f64> = HashMap::new();
            let mut evidence: HashMap<u64, Vec<u64>> = HashMap::new();
            // 相似任务
            let mut sims: Vec<(usize, f64)> = refs
                .iter()
                .enumerate()
                .map(|(ri, _)| (ri, cosine(tv, &corpus.vectors[ri])))
                .filter(|(_, s)| *s >= 0.15)
                .collect();
            sims.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (ri, s) in sims.into_iter().take(8) {
                let who = refs[ri].assignee_id.unwrap_or(0);
                *score.entry(who).or_default() += s;
                evidence.entry(who).or_default().push(refs[ri].id);
            }
            // 类别默认负责人
            let cat_owner = t
                .category_id
                .and_then(|cid| categories.iter().find(|c| c.id == cid))
                .and_then(|c| c.assigned_to.as_ref())
                .map(|a| a.id)
                .filter(|id| is_cand(*id));
            if let Some(owner) = cat_owner {
                *score.entry(owner).or_default() += 0.6;
            }
            // 分工方向
            let role = classify_role(&format!("{} {}", t.subject, t.description));
            let mut role_fit: Vec<u64> = Vec::new();
            if let Some(r) = role {
                for c in &cands {
                    if roles.get(&c.id).is_some_and(|mr| role_matches(mr, r)) {
                        *score.entry(c.id).or_default() += 0.25;
                        role_fit.push(c.id);
                    }
                }
            }
            // 负载惩罚
            for (who, s) in score.iter_mut() {
                *s -= 0.02 * load.get(who).copied().unwrap_or(0) as f64;
            }
            let best = score
                .iter()
                .filter(|(_, s)| **s > 0.05)
                .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(id, _)| *id);

            match best {
                Some(id) => {
                    let name = name_of(id).unwrap_or_else(|| format!("成员 #{id}"));
                    let (reason, source) = if cat_owner == Some(id) {
                        (format!("类别「{}」的默认负责人", t.category.as_deref().unwrap_or("")), "category")
                    } else if role_fit.contains(&id) && !evidence.contains_key(&id) {
                        let r = role.unwrap_or_default();
                        (format!("任务涉及{r}，{name} 负责{r}（当前 {} 个未关闭）", load.get(&id).copied().unwrap_or(0)), "role")
                    } else {
                        let ev: Vec<String> = evidence.get(&id).map(|v| v.iter().take(3).map(|i| format!("#{i}")).collect()).unwrap_or_default();
                        (format!("相似任务 {} 由 {name} 负责", ev.join("、")), "history")
                    };
                    AssignSuggestion { issue_id: t.id, assignee_id: Some(id), assignee_name: name, reason, source: source.into() }
                }
                None => match cands.iter().min_by_key(|c| (load.get(&c.id).copied().unwrap_or(0), c.id)) {
                    Some(c) => AssignSuggestion {
                        issue_id: t.id,
                        assignee_id: Some(c.id),
                        assignee_name: c.name.clone(),
                        reason: format!("没有相似的历史任务，{} 当前负载最低（{} 个未关闭）", c.name, load.get(&c.id).copied().unwrap_or(0)),
                        source: "workload".into(),
                    },
                    None => AssignSuggestion {
                        issue_id: t.id,
                        assignee_id: None,
                        assignee_name: String::new(),
                        reason: "无法读取项目成员，请手动选择".into(),
                        source: "workload".into(),
                    },
                },
            }
        })
        .collect()
}

/// 条目标记：1. / 1、/ 1) / (1) / 一、/ ① / - / * / •，以及 Markdown 待办 `- [ ]`
static ITEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(\d{1,2})([.、．)）])|[（(]\d{1,2}[)）]|[一二三四五六七八九十]{1,3}[、．.]|[①-⑳]|[-*•·+](?:\s+\[[ xX]\])?\s)\s*(.*)$")
        .expect("valid regex")
});
/// 列表前一行含这些词时，列表是操作步骤，不是可以分给不同人的事项
const STEP_WORDS: [&str; 6] = ["步骤", "复现", "重现", "操作流程", "steps", "reproduce"];

fn item_body(line: &str) -> Option<String> {
    let c = ITEM.captures(line)?;
    let body = c.get(3).map(|m| m.as_str().trim()).unwrap_or("");
    // “1.5 版本”这类小数不是条目
    if c.get(2).is_some_and(|m| m.as_str() == ".") && body.starts_with(|ch: char| ch.is_ascii_digit()) {
        return None;
    }
    Some(body.to_string())
}

/// 从描述中提取分条列出的事项，跳过“复现步骤”之类的列表。缩进的续行并入上一条。
pub fn split_items(text: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut header = String::new();
    let mut in_list = false;
    let mut skip = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            in_list = false;
            continue;
        }
        if let Some(body) = item_body(line) {
            if !in_list {
                let h = header.to_lowercase();
                skip = STEP_WORDS.iter().any(|w| h.contains(w));
                in_list = true;
            }
            if !skip && !body.is_empty() {
                items.push(body);
            }
        } else if in_list && raw.starts_with([' ', '\t', '　']) {
            if !skip {
                if let Some(last) = items.last_mut() {
                    last.push('\n');
                    last.push_str(line);
                }
            }
        } else {
            in_list = false;
            header = line.to_string();
        }
    }
    items.truncate(20);
    items
}

fn short_subject(item: &str) -> String {
    let first = item.lines().next().unwrap_or("").trim();
    truncate(first.trim_end_matches(['。', '；', ';', '，', ',', '：', ':']), 60)
}

/// 按条目拆分：每一条找最合适的负责人，按历史推荐给同一个人的条目合并成一个子任务。
/// 没有历史依据的条目各自成为子任务，按负载依次分给不同的人。
pub fn local_split(
    parent: &ProjectIssue,
    items: &[String],
    refs: &[ProjectIssue],
    members: &[Assignee],
    categories: &[IssueCategory],
    load: &HashMap<u64, usize>,
) -> Vec<SplitPart> {
    // 不带类别：原任务的类别默认负责人会把所有条目都拉到同一个人身上
    let targets: Vec<ProjectIssue> = items
        .iter()
        .map(|it| ProjectIssue { subject: short_subject(it), description: it.clone(), tracker: parent.tracker.clone(), ..Default::default() })
        .collect();
    // 原任务的描述包含所有条目，不能作为参考
    let refs: Vec<ProjectIssue> = refs.iter().filter(|r| r.id != parent.id).cloned().collect();
    let suggestions = local_suggestions(&targets, &refs, members, categories, load);
    let cands = candidates(members);
    let mut planned: HashMap<u64, usize> = HashMap::new();

    let mut groups: Vec<(Vec<usize>, AssignSuggestion)> = Vec::new();
    for (i, mut s) in suggestions.into_iter().enumerate() {
        if s.source == "workload" {
            // 把这次拆分已经分出去的条目也算进负载，避免全部落到同一个人身上
            if let Some(c) = cands.iter().min_by_key(|c| (load.get(&c.id).copied().unwrap_or(0) + planned.get(&c.id).copied().unwrap_or(0), c.id)) {
                s.assignee_id = Some(c.id);
                s.assignee_name = c.name.clone();
                s.reason = format!("没有相似的历史任务，按负载分配（{} 当前 {} 个未关闭）", c.name, load.get(&c.id).copied().unwrap_or(0));
            }
            if let Some(id) = s.assignee_id {
                *planned.entry(id).or_default() += 1;
            }
            groups.push((vec![i], s));
            continue;
        }
        if let Some(id) = s.assignee_id {
            *planned.entry(id).or_default() += 1;
        }
        match groups.iter_mut().find(|(_, g)| g.source != "workload" && s.assignee_id.is_some() && g.assignee_id == s.assignee_id) {
            Some((idx, _)) => idx.push(i),
            None => groups.push((vec![i], s)),
        }
    }
    groups
        .into_iter()
        .map(|(idx, s)| {
            let nums: Vec<String> = idx.iter().map(|i| (i + 1).to_string()).collect();
            let subject = if idx.len() == 1 {
                short_subject(&items[idx[0]])
            } else {
                format!("{}（第 {} 项）", truncate(&parent.subject, 40), nums.join("、"))
            };
            let description = idx.iter().map(|i| format!("{}. {}", i + 1, items[*i])).collect::<Vec<_>>().join("\n");
            SplitPart {
                subject,
                description,
                assignee_id: s.assignee_id,
                assignee_name: s.assignee_name,
                reason: if idx.len() > 1 { format!("第 {} 项：{}", nums.join("、"), s.reason) } else { s.reason },
                source: s.source,
                ..Default::default()
            }
        })
        .collect()
}

// ---------------- 分派：分工方向、多人方案、默认期限 ----------------

/// 可选的分工方向
pub const ROLES: [&str; 7] = ["前端", "后端", "全栈", "测试", "运维", "数据", "产品"];

/// 方向关键词：(方向, 强信号, 弱信号)。每个词只计一次，强信号 2 分、弱信号 1 分
const ROLE_WORDS: [(&str, &[&str], &[&str]); 5] = [
    (
        "前端",
        &["前端", "样式", "布局", "按钮", "弹窗", "弹框", "交互", "界面", "css", "错位", "对齐", "字体", "图标", "小程序", "h5", "移动端", "输入框", "下拉框", "前台", "vue", "react"],
        &["页面", "显示", "展示", "列表", "点击", "菜单", "跳转", "浏览器", "表单"],
    ),
    (
        "后端",
        &["后端", "接口", "api", "数据库", "sql", "服务端", "定时任务", "存储过程", "索引", "空指针", "并发", "缓存", "队列"],
        &["导出", "导入", "字段", "查询", "计算", "报错", "超时", "日志", "500", "性能", "同步"],
    ),
    ("测试", &["测试", "回归", "用例", "冒烟"], &["验证", "复测"]),
    ("运维", &["部署", "服务器", "运维", "证书", "域名", "nginx", "磁盘", "宕机", "防火墙", "备份", "扩容"], &["重启", "网络", "环境", "内存", "端口"]),
    ("数据", &["数据修复", "修改数据", "刷数据", "补数据", "删除数据", "数据迁移", "跑脚本", "数据处理"], &["报表", "统计", "口径"]),
];

/// 各方向的得分和命中的词，按得分从高到低
pub fn role_scores(text: &str) -> Vec<(&'static str, u32, Vec<&'static str>)> {
    let t = text.to_lowercase();
    let mut out: Vec<(&'static str, u32, Vec<&'static str>)> = ROLE_WORDS
        .iter()
        .map(|(role, strong, weak)| {
            let mut hits = Vec::new();
            let mut score = 0;
            for w in strong.iter() {
                if t.contains(w) {
                    score += 2;
                    hits.push(*w);
                }
            }
            for w in weak.iter() {
                if t.contains(w) {
                    score += 1;
                    hits.push(*w);
                }
            }
            (*role, score, hits)
        })
        .filter(|(_, score, _)| *score > 0)
        .collect();
    out.sort_by_key(|x| std::cmp::Reverse(x.1));
    out
}

/// 文本主要涉及的方向：最高分至少 2 分且明显高于第二名
pub fn classify_role(text: &str) -> Option<&'static str> {
    let s = role_scores(text);
    let top = s.first()?;
    let second = s.get(1).map(|x| x.1).unwrap_or(0);
    (top.1 >= 2 && top.1 > second).then_some(top.0)
}

/// 成员的分工能否承担某个方向（全栈可以做前端和后端）
pub fn role_matches(member_role: &str, role: &str) -> bool {
    member_role == role || (member_role == "全栈" && (role == "前端" || role == "后端"))
}

/// Redmine 角色名里带方向的（如“前端开发”），直接作为分工
pub fn role_from_names(roles: &[String]) -> Option<&'static str> {
    roles.iter().find_map(|r| {
        let r = r.to_lowercase();
        if r.contains("前端") || r.contains("frontend") {
            Some("前端")
        } else if r.contains("后端") || r.contains("backend") {
            Some("后端")
        } else if r.contains("全栈") || r.contains("fullstack") {
            Some("全栈")
        } else if r.contains("测试") || r.contains("qa") {
            Some("测试")
        } else if r.contains("运维") || r.contains("devops") {
            Some("运维")
        } else {
            None
        }
    })
}

/// 按历史任务推断成员分工：近期负责的任务大多涉及某个方向。返回 成员 ID → (方向, 依据)
pub fn guess_roles(members: &[Assignee], refs: &[ProjectIssue]) -> HashMap<u64, (String, String)> {
    let mut out = HashMap::new();
    for m in candidates(members) {
        let mine: Vec<&ProjectIssue> = refs.iter().filter(|r| r.assignee_id == Some(m.id)).collect();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for r in &mine {
            let text = format!("{} {} {}", r.subject, r.category.as_deref().unwrap_or(""), truncate(&r.description, 200));
            if let Some(role) = classify_role(&text) {
                *counts.entry(role).or_default() += 1;
            }
        }
        let classified: usize = counts.values().sum();
        if classified < 3 {
            continue;
        }
        let fe = counts.get("前端").copied().unwrap_or(0);
        let be = counts.get("后端").copied().unwrap_or(0);
        let (role, n) = if fe >= 3 && be >= 3 && fe * 10 >= classified * 3 && be * 10 >= classified * 3 {
            ("全栈", fe + be)
        } else {
            match counts.iter().max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0))) {
                Some((r, n)) if *n >= 3 && n * 2 >= classified => (*r, *n),
                _ => continue,
            }
        };
        let what = if role == "全栈" { "前端和后端".to_string() } else { role.to_string() };
        out.insert(m.id, (role.to_string(), format!("近期负责的 {} 个任务中有 {n} 个涉及{what}", mine.len())));
    }
    out
}

/// 判断一个任务是否需要多人分工，需要时给出每人一个子任务的方案。
///
/// - 描述分条列出事项时：按方向给每条归类，涉及两个以上方向且都有对应成员时，按方向拆；
///   成员分工未知时，退回按历史分工拆（每一部分都要有历史依据）
/// - 没有分条时：标题和描述同时明显涉及两个方向（各至少 3 分）才拆
pub fn multi_plan(
    target: &ProjectIssue,
    items: &[String],
    refs: &[ProjectIssue],
    members: &[Assignee],
    categories: &[IssueCategory],
    load: &HashMap<u64, usize>,
    roles: &HashMap<u64, String>,
) -> Option<MultiPlan> {
    let cands = candidates(members);
    let refs: Vec<ProjectIssue> = refs.iter().filter(|r| r.id != target.id).cloned().collect();
    let has_member = |role: &str| cands.iter().any(|c| roles.get(&c.id).is_some_and(|r| role_matches(r, role)));

    // 每个方向：(方向, 条目序号)
    let mut groups: Vec<(&'static str, Vec<usize>)> = Vec::new();
    let mut unknown: Vec<usize> = Vec::new();
    let mut whole_hits: HashMap<&str, Vec<&str>> = HashMap::new();
    if items.len() >= 2 {
        for (i, it) in items.iter().enumerate() {
            match classify_role(it).filter(|r| has_member(r)) {
                Some(r) => match groups.iter_mut().find(|g| g.0 == r) {
                    Some(g) => g.1.push(i),
                    None => groups.push((r, vec![i])),
                },
                None => unknown.push(i),
            }
        }
    } else {
        for (role, score, hits) in role_scores(&format!("{} {}", target.subject, target.description)) {
            if score >= 3 && has_member(role) && groups.len() < 3 {
                groups.push((role, vec![]));
                whole_hits.insert(role, hits);
            }
        }
    }

    if groups.len() >= 2 {
        let mut chosen: Vec<(&str, Vec<usize>, AssignSuggestion)> = Vec::new();
        for (role, idx) in &groups {
            let text = if idx.is_empty() {
                format!("{} {}", target.subject, target.description)
            } else {
                idx.iter().map(|i| items[*i].as_str()).collect::<Vec<_>>().join("\n")
            };
            let taken: Vec<u64> = chosen.iter().filter_map(|c| c.2.assignee_id).collect();
            let pool: Vec<Assignee> = cands
                .iter()
                .filter(|c| roles.get(&c.id).is_some_and(|r| role_matches(r, role)))
                .map(|c| (*c).clone())
                .collect();
            // 同一个人不重复承担两个方向（有其他人选时）
            let free: Vec<Assignee> = pool.iter().filter(|c| !taken.contains(&c.id)).cloned().collect();
            let pool = if free.is_empty() { pool } else { free };
            let t = ProjectIssue { subject: short_subject(&text), description: text, tracker: target.tracker.clone(), ..Default::default() };
            let mut s = suggest_with_roles(&[t], &refs, &pool, categories, load, roles).remove(0);
            if s.source == "workload" || s.source == "role" {
                let n = load.get(&s.assignee_id.unwrap_or(0)).copied().unwrap_or(0);
                s.reason = format!("{} 负责{role}（当前 {n} 个未关闭）", s.assignee_name);
            }
            chosen.push((role, idx.clone(), s));
        }
        let distinct: std::collections::HashSet<u64> = chosen.iter().filter_map(|c| c.2.assignee_id).collect();
        if distinct.len() >= 2 {
            // 没有方向的条目放到按历史最合适的那一部分，否则放到第一部分
            for i in unknown {
                let pool: Vec<Assignee> = members.iter().filter(|m| distinct.contains(&m.id)).cloned().collect();
                let t = ProjectIssue { subject: short_subject(&items[i]), description: items[i].clone(), ..Default::default() };
                let s = local_suggestions(&[t], &refs, &pool, &[], load).remove(0);
                let at = chosen
                    .iter()
                    .position(|c| s.source == "history" && c.2.assignee_id == s.assignee_id)
                    .unwrap_or(0);
                chosen[at].1.push(i);
            }
            let names: Vec<&str> = chosen.iter().map(|c| c.0).collect();
            let reason = if items.len() >= 2 {
                format!("描述里的事项分属{}", names.join("、"))
            } else {
                let parts: Vec<String> = chosen
                    .iter()
                    .map(|c| format!("{}（{}）", c.0, whole_hits.get(c.0).map(|h| h.iter().take(3).copied().collect::<Vec<_>>().join("、")).unwrap_or_default()))
                    .collect();
                format!("描述同时涉及{}", parts.join("和"))
            };
            let parts = chosen
                .into_iter()
                .map(|(role, mut idx, s)| {
                    idx.sort_unstable();
                    let nums: Vec<String> = idx.iter().map(|i| (i + 1).to_string()).collect();
                    let description = if idx.is_empty() {
                        format!("负责{role}部分，详见原任务 #{}。", target.id)
                    } else {
                        idx.iter().map(|i| format!("{}. {}", i + 1, items[*i])).collect::<Vec<_>>().join("\n")
                    };
                    SplitPart {
                        subject: format!("{}（{role}）", truncate(&target.subject, 50)),
                        description,
                        assignee_id: s.assignee_id,
                        assignee_name: s.assignee_name,
                        reason: if nums.is_empty() { s.reason } else { format!("第 {} 项：{}", nums.join("、"), s.reason) },
                        source: "role".into(),
                        role: role.to_string(),
                        ..Default::default()
                    }
                })
                .collect();
            return Some(MultiPlan { parts, reason, source: "role".into() });
        }
    }

    // 成员分工未知：按历史分工拆，每一部分都要有历史或类别依据
    if items.len() >= 2 {
        let parts = local_split(target, items, &refs, members, categories, load);
        let distinct: std::collections::HashSet<u64> = parts.iter().filter_map(|p| p.assignee_id).collect();
        if parts.len() >= 2 && distinct.len() == parts.len() && parts.iter().all(|p| p.source == "history" || p.source == "category") {
            return Some(MultiPlan { parts, reason: "按历史分工，这些事项通常由不同的人负责".into(), source: "history".into() });
        }
    }
    None
}

/// 优先级档位：urgent | high | normal | low
pub fn priority_tier(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if ["立刻", "紧急", "urgent", "immediate"].iter().any(|w| n.contains(w)) {
        "urgent"
    } else if n.contains('高') || n.contains("high") {
        "high"
    } else if n.contains('低') || n.contains("low") {
        "low"
    } else {
        "normal"
    }
}

/// 从 `from` 起往后数 n 个工作日（跳过周六周日）
pub fn add_workdays(from: NaiveDate, n: u32) -> NaiveDate {
    let mut d = from;
    let mut left = n;
    while left > 0 {
        d += chrono::Duration::days(1);
        if d.weekday().num_days_from_monday() < 5 {
            left -= 1;
        }
    }
    d
}

/// 按优先级给的默认截止日期，该档位设为 0 时不设置
pub fn default_due(priority: &str, days: &DueDays, today: NaiveDate) -> Option<NaiveDate> {
    let n = match priority_tier(priority) {
        "urgent" => days.urgent,
        "high" => days.high,
        "low" => days.low,
        _ => days.normal,
    };
    (n > 0).then(|| add_workdays(today, n))
}

/// 视为“待分派”的状态：设置里指定的；没有指定时找“新建”一类的状态，都没有就取第一个未关闭状态
pub fn pending_statuses(statuses: &[NamedId], configured: &[u64]) -> Vec<NamedId> {
    if !configured.is_empty() {
        return statuses.iter().filter(|s| configured.contains(&s.id)).cloned().collect();
    }
    const WORDS: [&str; 10] = ["新建", "新提交", "待分", "待处理", "待指派", "未开始", "已提交", "未处理", "new", "open"];
    let open: Vec<&NamedId> = statuses.iter().filter(|s| s.is_closed != Some(true)).collect();
    let matched: Vec<NamedId> = open
        .iter()
        .filter(|s| {
            let n = s.name.to_lowercase();
            n == "新" || WORDS.iter().any(|w| n.contains(w))
        })
        .map(|s| (*s).clone())
        .collect();
    if matched.is_empty() {
        open.first().map(|s| vec![(*s).clone()]).unwrap_or_default()
    } else {
        matched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NamedId;

    fn issue(id: u64, subject: &str, assignee: Option<u64>, category: Option<u64>) -> ProjectIssue {
        ProjectIssue {
            id,
            subject: subject.into(),
            assignee_id: assignee,
            category_id: category,
            category: category.map(|_| "报表".into()),
            ..Default::default()
        }
    }

    fn member(id: u64, name: &str, roles: &[&str]) -> Assignee {
        Assignee { id, name: name.into(), is_group: false, roles: roles.iter().map(|r| r.to_string()).collect() }
    }

    #[test]
    fn detects_manager_roles() {
        assert!(is_manager_role("项目经理"));
        assert!(is_manager_role("Manager"));
        assert!(!is_manager_role("开发人员"));
    }

    #[test]
    fn suggests_by_category_history_and_workload() {
        let members = vec![
            member(1, "张三", &["开发"]),
            member(2, "李四", &["开发"]),
            member(3, "王五", &["报告人"]),
            member(4, "赵六", &["开发"]),
        ];
        let refs = vec![
            issue(10, "订单导出 Excel 报错", Some(1), None),
            issue(11, "订单导出超时", Some(1), None),
            issue(12, "财务报表金额不对", Some(2), None),
            issue(13, "权限开通", Some(2), None),
            issue(14, "权限调整", Some(2), None),
        ];
        let targets = vec![
            issue(20, "导出订单 Excel 时报错 500", None, None),
            issue(21, "月度报表口径确认", None, Some(7)),
            issue(22, "会议室预约系统打不开", None, None),
        ];
        let cats = vec![IssueCategory { id: 7, name: "报表".into(), assigned_to: Some(NamedId { id: 4, name: "赵六".into(), is_closed: None }) }];
        let load = HashMap::from([(1, 2), (2, 3), (4, 0)]);
        let s = local_suggestions(&targets, &refs, &members, &cats, &load);
        assert_eq!((s[0].assignee_id, s[0].source.as_str()), (Some(1), "history"));
        assert!(s[0].reason.contains("#10"));
        assert_eq!((s[1].assignee_id, s[1].source.as_str()), (Some(4), "category"));
        // 没有相似任务时给负载最低的（报告人不参与）
        assert_eq!((s[2].assignee_id, s[2].source.as_str()), (Some(4), "workload"));
    }

    #[test]
    fn extracts_items_but_skips_steps() {
        let text = "需要调整以下内容：\n1. 订单列表增加导出按钮\n2、导出接口支持按月份筛选\n   月份默认为当月\n（3）导出文件名带上日期\n\n复现步骤：\n1. 打开订单列表\n2. 点击导出\n\n版本 1.5 之后出现";
        let items = split_items(text);
        assert_eq!(items, vec!["订单列表增加导出按钮", "导出接口支持按月份筛选\n月份默认为当月", "导出文件名带上日期"]);
        assert_eq!(split_items("- [ ] 前端改按钮\n- [x] 后端加字段\n* 补测试"), vec!["前端改按钮", "后端加字段", "补测试"]);
        assert!(split_items("1.5 版本以后报错\n2.0 正常").is_empty());
    }

    #[test]
    fn local_split_groups_items_by_assignee() {
        let members = vec![member(1, "张三", &["开发"]), member(2, "李四", &["开发"])];
        let refs = vec![
            issue(10, "订单列表页面按钮样式错乱", Some(1), None),
            issue(11, "订单列表页面分页组件", Some(1), None),
            issue(12, "导出接口超时", Some(2), None),
            issue(13, "导出接口增加字段", Some(2), None),
            // 原任务本身不能作为参考
            issue(99, "订单列表页面 导出接口 导出页面", Some(2), None),
        ];
        let parent = ProjectIssue { id: 99, subject: "订单导出优化".into(), ..Default::default() };
        let items: Vec<String> = ["订单列表页面增加导出按钮", "导出接口支持按月份筛选", "订单列表页面显示导出进度"].map(String::from).to_vec();
        let parts = local_split(&parent, &items, &refs, &members, &[], &HashMap::new());
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].assignee_id, Some(1));
        assert_eq!(parts[0].subject, "订单导出优化（第 1、3 项）");
        assert!(parts[0].description.contains("3. 订单列表页面显示导出进度"));
        assert_eq!((parts[1].assignee_id, parts[1].subject.as_str()), (Some(2), "导出接口支持按月份筛选"));

        // 没有历史依据的条目不合并，按负载依次分给不同的人
        let items: Vec<String> = ["会议室预约改版", "门禁卡补办流程", "食堂菜单发布"].map(String::from).to_vec();
        let load = HashMap::from([(1, 1), (2, 0)]);
        let parts = local_split(&parent, &items, &refs, &members, &[], &load);
        let who: Vec<Option<u64>> = parts.iter().map(|p| p.assignee_id).collect();
        assert_eq!(who, vec![Some(2), Some(1), Some(2)]);
        assert!(parts.iter().all(|p| p.source == "workload"));
    }

    #[test]
    fn classifies_roles_and_detects_multi_person_work() {
        assert_eq!(classify_role("订单列表页面按钮样式错位"), Some("前端"));
        assert_eq!(classify_role("导出接口超时，数据库慢查询"), Some("后端"));
        assert_eq!(classify_role("麻烦看一下"), None);

        let members = vec![member(1, "周岩", &["开发"]), member(2, "陈默", &["开发"]), member(3, "许诺", &["开发"])];
        let roles: HashMap<u64, String> = HashMap::from([(1, "前端".into()), (2, "后端".into()), (3, "后端".into())]);
        let refs = vec![issue(10, "导出接口增加字段", Some(2), None), issue(11, "导出接口超时", Some(2), None)];
        let load = HashMap::from([(1, 2), (2, 4), (3, 1)]);

        // 分条列出、分属前端和后端
        let parent = ProjectIssue { id: 99, subject: "订单导出优化".into(), ..Default::default() };
        let items: Vec<String> = ["订单列表页面增加导出按钮", "导出接口支持按月份筛选", "补充一下说明"].map(String::from).to_vec();
        let plan = multi_plan(&parent, &items, &refs, &members, &[], &load, &roles).expect("需要两个人");
        assert_eq!(plan.source, "role");
        assert_eq!(plan.parts.len(), 2);
        assert_eq!((plan.parts[0].role.as_str(), plan.parts[0].assignee_id), ("前端", Some(1)));
        // 后端选做过导出接口的陈默，而不是负载更低的许诺
        assert_eq!((plan.parts[1].role.as_str(), plan.parts[1].assignee_id), ("后端", Some(2)));
        assert_eq!(plan.parts[0].subject, "订单导出优化（前端）");
        assert!(plan.parts.iter().any(|p| p.description.contains("3. 补充一下说明")));

        // 没有分条：标题和描述同时明显涉及前端和后端
        let t = ProjectIssue { id: 98, subject: "客户列表页面按钮错位，导出接口也超时".into(), ..Default::default() };
        let plan = multi_plan(&t, &[], &refs, &members, &[], &load, &roles).expect("需要两个人");
        assert_eq!(plan.parts.len(), 2);
        assert!(plan.reason.contains("前端") && plan.reason.contains("后端"));
        assert!(plan.parts[0].description.contains("#98"));

        // 只涉及一个方向
        let t = ProjectIssue { id: 97, subject: "登录按钮样式错位".into(), ..Default::default() };
        assert!(multi_plan(&t, &[], &refs, &members, &[], &load, &roles).is_none());

        // 单人推荐：任务涉及前端时优先前端成员
        let s = suggest_with_roles(&[t], &refs, &members, &[], &load, &roles);
        assert_eq!((s[0].assignee_id, s[0].source.as_str()), (Some(1), "role"));
    }

    #[test]
    fn guesses_member_roles_from_history() {
        let members = vec![member(1, "周岩", &["开发"]), member(2, "陈默", &["开发"])];
        let mut refs = Vec::new();
        for (i, s) in ["列表页面按钮错位", "弹窗样式调整", "表单输入框校验提示", "首页布局优化"].iter().enumerate() {
            refs.push(issue(10 + i as u64, s, Some(1), None));
        }
        refs.push(issue(20, "导出接口超时", Some(2), None));
        let g = guess_roles(&members, &refs);
        assert_eq!(g.get(&1).map(|x| x.0.as_str()), Some("前端"));
        assert!(g.get(&1).unwrap().1.contains("4 个"));
        assert!(!g.contains_key(&2), "依据不足时不推断");
        assert_eq!(role_from_names(&["前端开发".into()]), Some("前端"));
    }

    #[test]
    fn computes_default_due_dates_and_pending_statuses() {
        let fri = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let days = DueDays { urgent: 1, high: 3, normal: 5, low: 0 };
        assert_eq!(default_due("立刻", &days, fri), NaiveDate::from_ymd_opt(2026, 11, 9)); // 跳过周末
        assert_eq!(default_due("高", &days, fri), NaiveDate::from_ymd_opt(2026, 11, 11));
        assert_eq!(default_due("普通", &days, fri), NaiveDate::from_ymd_opt(2026, 11, 13));
        assert_eq!(default_due("低", &days, fri), None);

        let st = |id, name: &str, closed| NamedId { id, name: name.into(), is_closed: Some(closed) };
        let statuses = vec![st(1, "新建", false), st(2, "进行中", false), st(4, "反馈", false), st(5, "已关闭", true)];
        assert_eq!(pending_statuses(&statuses, &[]).iter().map(|s| s.id).collect::<Vec<_>>(), vec![1]);
        assert_eq!(pending_statuses(&statuses, &[2, 4]).len(), 2);
        let custom = vec![st(7, "待确认", false), st(8, "开发中", false)];
        assert_eq!(pending_statuses(&custom, &[])[0].id, 7);
    }

    #[test]
    fn weeks_end_with_current_week() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(); // 周三
        let w = recent_weeks(today, 8);
        assert_eq!(w.len(), 8);
        assert_eq!(w[7].0, NaiveDate::from_ymd_opt(2026, 11, 2).unwrap());
        assert_eq!(w[0].0, NaiveDate::from_ymd_opt(2026, 9, 14).unwrap());
    }
}
