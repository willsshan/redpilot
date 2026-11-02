//! 发送给 AI 之前的敏感信息脱敏。

use regex::Regex;
use std::sync::LazyLock;

struct Rule {
    re: Regex,
    replacement: &'static str,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let r = |p: &str, replacement: &'static str| Rule {
        re: Regex::new(p).expect("valid regex"),
        replacement,
    };
    vec![
        // 密码 / 令牌类键值对：password=xxx、密码：xxx、token: xxx
        r(r"(?i)((?:password|passwd|pwd|secret|token|api[_-]?key|密码|口令|密钥)\s*[:：=]\s*)[^\s，,；;。]+", "${1}[已脱敏]"),
        // 身份证号（18 位）
        r(r"(?-u:\b)\d{17}[\dXx](?-u:\b)", "[身份证]"),
        // 银行卡号（16-19 位）
        r(r"(?-u:\b)\d{16,19}(?-u:\b)", "[卡号]"),
        // 手机号
        r(r"(?-u:\b)1[3-9]\d{9}(?-u:\b)", "[手机号]"),
        // 邮箱
        r(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", "[邮箱]"),
        // IPv4
        r(r"(?-u:\b)(?:\d{1,3}\.){3}\d{1,3}(?-u:\b)", "[IP]"),
        // 长随机串（JWT、AK/SK 等）
        r(r"(?-u:\b)[A-Za-z0-9_\-]{32,}(?-u:\b)", "[密钥串]"),
    ]
});

pub fn mask(text: &str) -> String {
    let mut out = text.to_string();
    for rule in RULES.iter() {
        out = rule.re.replace_all(&out, rule.replacement).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::mask;

    #[test]
    fn masks_common_sensitive_data() {
        let s = mask("联系 13812345678，服务器 10.0.0.12，密码：Abc123!，邮箱 a.b@corp.com");
        assert!(s.contains("[手机号]"));
        assert!(s.contains("[IP]"));
        assert!(s.contains("密码：[已脱敏]"));
        assert!(s.contains("[邮箱]"));
        assert!(!s.contains("13812345678"));
    }
}
