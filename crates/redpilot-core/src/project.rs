//! 项目总览：项目视角的任务、进度统计，以及转派时的负责人推荐（本地算法，不消耗 token）。

use crate::analysis::{cosine, Corpus};
use crate::models::{AssignSuggestion, Assignee, Issue, ProjectIssue};
use crate::redmine::{truncate, IssueCategory};
use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;

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
    fn weeks_end_with_current_week() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(); // 周三
        let w = recent_weeks(today, 8);
        assert_eq!(w.len(), 8);
        assert_eq!(w[7].0, NaiveDate::from_ymd_opt(2026, 11, 2).unwrap());
        assert_eq!(w[0].0, NaiveDate::from_ymd_opt(2026, 9, 14).unwrap());
    }
}
