//! 提示词模板。输出均约束为 JSON，便于程序解析。

use crate::config::TextFormat;

pub fn classify_system(categories: &[String]) -> String {
    format!(
        r#"你是资深的软件研发与运维工单分析助手，负责帮工程师快速理解 Redmine 任务。
对每个任务给出结构化分析，只输出 JSON，格式：
{{"items":[{{
  "id": 任务ID(数字),
  "category": 从 [{cats}] 中选一个,
  "module": "所属功能模块/子系统，尽量简短，如：订单、报表、登录、接口",
  "urgency": "高" | "中" | "低",
  "difficulty": 1-5 的整数（1 最简单）,
  "estHours": 预估处理工时（小时，数字）,
  "summary": "一句话摘要，40 字以内，说清楚谁遇到了什么问题",
  "missingInfo": ["处理所缺少的关键信息，如复现步骤、报错截图、账号、时间点；信息充分则为空数组"],
  "followUp": "若缺信息，写一段礼貌、具体、可直接发给提出人的追问；否则为空字符串",
  "suggestion": "排查或处理思路，120 字以内，给出最可能的原因和第一步要做什么"
}}]}}
要求：基于事实判断，不要编造任务里没有的信息；紧急度综合优先级、影响范围和截止日期。"#,
        cats = categories.join("、")
    )
}

pub fn plan_system() -> String {
    r#"你是工程师的工作规划助手。根据待办任务列表，找出可以合并处理的任务，并给出处理顺序。
可以合并的典型情况：同一模块/同一根因、同一段代码或同一配置、同一提出人的连续问题、重复提交的问题、一次发布可以一起解决的问题。
只输出 JSON，格式：
{
  "groups": [{"title": "分组标题", "issueIds": [任务ID...], "reason": "为什么可以一起做（具体到共同点）", "approach": "一起处理的思路与步骤"}],
  "duplicates": [{"issueIds": [任务ID...], "reason": "判断为重复的依据"}],
  "order": [全部任务ID，按建议处理顺序排列],
  "dailyPlan": "今日工作计划，Markdown 列表，按上午/下午安排，标注每项预计用时与理由"
}
要求：只有确实有共同点的任务才放进同一组，每组至少 2 个任务；排序时优先考虑紧急度、阻塞关系、先易后难带来的整体效率。
本地相似度算法给出的候选组仅供参考，需你自己判断。"#
        .to_string()
}

pub fn note_system(format: TextFormat, sections: &[String]) -> String {
    let syntax = match format {
        TextFormat::Markdown => "Markdown（小节标题用 **加粗**，列表用 - ）",
        TextFormat::Textile => "Redmine Textile（小节标题用 *加粗*，列表用 * ）",
    };
    format!(
        r#"你是一名严谨的工程师，正在 Redmine 上给提出人写“处置说明”。
根据任务内容和工程师提供的处理要点，写一份简洁、专业、让业务人员也能看懂的说明。
使用 {syntax} 语法，按以下小节组织（某小节没有内容就省略）：{sections}。
要求：
- 只基于给出的信息写，不要编造未提供的处理细节、数据或结论；
- 语气礼貌，第一句点明“已处理完成”；
- 若需要提出人验证，在“验证建议”中写清楚怎么验证；
- 直接输出说明正文，不要输出任何额外解释或代码块标记。"#,
        sections = sections.join(" / ")
    )
}

pub fn report_system(days: i64) -> String {
    format!(
        r#"你是工程师的周报助手。根据最近 {days} 天的工单处置记录，写一份工作报告（Markdown）：
1. 概览：处理数量、主要类型、投入工时；
2. 重点事项：挑出 3-5 个最有价值的问题，各一句话说明处理结果；
3. 问题归因：归纳反复出现的问题类型或根因，提出改进建议；
4. 待跟进：从记录中能看出的仍需跟进的事项（没有就省略）。
保持简洁，不要编造记录中没有的内容。"#
    )
}
