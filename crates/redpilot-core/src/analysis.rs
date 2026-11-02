//! 本地分析：关键词规则分类、TF-IDF 文本相似度、候选分组。
//! 全部离线完成、不消耗 token，用于给 AI 做预筛和兜底。

use crate::config::CategoryRule;
use std::collections::{HashMap, HashSet};

/// 关键词规则分类，命中关键词最多的分类胜出
pub fn rule_classify(text: &str, rules: &[CategoryRule]) -> Option<String> {
    let lower = text.to_lowercase();
    let mut best: Option<(&str, usize)> = None;
    for rule in rules {
        let hits = rule
            .keywords
            .iter()
            .filter(|k| !k.is_empty() && lower.contains(&k.to_lowercase()))
            .count();
        if hits > 0 && best.map_or(true, |(_, h)| hits > h) {
            best = Some((&rule.category, hits));
        }
    }
    best.map(|(c, _)| c.to_string())
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}

/// 分词：中文用字二元组（无需词典，适合短文本相似度），英文/数字按单词
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut prev_cjk: Option<char> = None;
    for c in text.chars() {
        if is_cjk(c) {
            if !word.is_empty() {
                push_word(&mut tokens, &mut word);
            }
            if let Some(p) = prev_cjk {
                tokens.push(format!("{p}{c}"));
            }
            prev_cjk = Some(c);
        } else if c.is_alphanumeric() || c == '_' {
            prev_cjk = None;
            word.extend(c.to_lowercase());
        } else {
            prev_cjk = None;
            push_word(&mut tokens, &mut word);
        }
    }
    push_word(&mut tokens, &mut word);
    tokens
}

fn push_word(tokens: &mut Vec<String>, word: &mut String) {
    if word.chars().count() >= 2 && !word.chars().all(|c| c.is_ascii_digit()) {
        tokens.push(std::mem::take(word));
    } else {
        word.clear();
    }
}

/// 稀疏 TF-IDF 向量（已归一化）
pub type Vector = HashMap<String, f64>;

pub struct Corpus {
    pub vectors: Vec<Vector>,
}

impl Corpus {
    pub fn build(docs: &[String]) -> Self {
        let tokenized: Vec<Vec<String>> = docs.iter().map(|d| tokenize(d)).collect();
        let n = tokenized.len().max(1) as f64;
        let mut df: HashMap<&str, usize> = HashMap::new();
        for toks in &tokenized {
            let uniq: HashSet<&str> = toks.iter().map(String::as_str).collect();
            for t in uniq {
                *df.entry(t).or_default() += 1;
            }
        }
        let vectors = tokenized
            .iter()
            .map(|toks| {
                let mut tf: HashMap<String, f64> = HashMap::new();
                for t in toks {
                    *tf.entry(t.clone()).or_default() += 1.0;
                }
                let mut v: Vector = tf
                    .into_iter()
                    .map(|(t, f)| {
                        let idf = (n / (1.0 + *df.get(t.as_str()).unwrap_or(&0) as f64)).ln() + 1.0;
                        (t, (1.0 + f.ln()) * idf)
                    })
                    .collect();
                let norm = v.values().map(|x| x * x).sum::<f64>().sqrt();
                if norm > 0.0 {
                    v.values_mut().for_each(|x| *x /= norm);
                }
                v
            })
            .collect();
        Self { vectors }
    }
}

pub fn cosine(a: &Vector, b: &Vector) -> f64 {
    let (small, large) = if a.len() < b.len() { (a, b) } else { (b, a) };
    small
        .iter()
        .filter_map(|(k, x)| large.get(k).map(|y| x * y))
        .sum()
}

/// 与第 `target` 篇最相似的文档下标及分数（降序）
pub fn most_similar(corpus: &Corpus, target: usize, limit: usize, min_score: f64) -> Vec<(usize, f64)> {
    let t = &corpus.vectors[target];
    let mut scored: Vec<(usize, f64)> = corpus
        .vectors
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != target)
        .map(|(i, v)| (i, cosine(t, v)))
        .filter(|(_, s)| *s >= min_score)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

/// 基于相似度阈值的并查集聚类，只返回 2 个以上成员的候选组
pub fn pre_cluster(corpus: &Corpus, threshold: f64) -> Vec<Vec<usize>> {
    let n = corpus.vectors.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut Vec<usize>, x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let next = p[c];
            p[c] = r;
            c = next;
        }
        r
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if cosine(&corpus.vectors[i], &corpus.vectors[j]) >= threshold {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() > 1).collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_rules;

    #[test]
    fn classifies_by_keywords() {
        assert_eq!(rule_classify("登录后页面报错 500", &default_rules()).as_deref(), Some("缺陷"));
        assert_eq!(rule_classify("请问如何导出", &default_rules()).as_deref(), Some("咨询"));
    }

    #[test]
    fn similar_texts_cluster_together() {
        let docs = vec![
            "订单导出 Excel 报错".to_string(),
            "导出订单 Excel 时报错".to_string(),
            "申请开通财务系统权限".to_string(),
        ];
        let c = Corpus::build(&docs);
        let groups = pre_cluster(&c, 0.3);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
        assert!(!groups[0].contains(&2));
    }
}
