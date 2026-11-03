//! 项目总览：项目视角的任务、进度统计，以及转派时的负责人推荐（本地算法，不消耗 token）。

use crate::analysis::{cosine, Corpus};
use crate::models::{AssignSuggestion, Assignee, Issue, ProjectIssue, SplitPart};
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
    fn weeks_end_with_current_week() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(); // 周三
        let w = recent_weeks(today, 8);
        assert_eq!(w.len(), 8);
        assert_eq!(w[7].0, NaiveDate::from_ymd_opt(2026, 11, 2).unwrap());
        assert_eq!(w[0].0, NaiveDate::from_ymd_opt(2026, 9, 14).unwrap());
    }
}
