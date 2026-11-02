//! OpenAI 兼容的对话接口客户端，默认对接 DeepSeek。

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone)]
pub struct AiClient {
    http: reqwest::Client,
    endpoint: String,
    key: String,
    pub model: String,
    json_mode: bool,
    temperature: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(c: impl Into<String>) -> Self {
        Self { role: "system".into(), content: c.into() }
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self { role: "user".into(), content: c.into() }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
}

impl AiClient {
    pub fn new(base_url: &str, key: &str, model: &str, json_mode: bool, temperature: f32, proxy: &str) -> Result<Self> {
        let base = base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            bail!("尚未配置 AI 接口地址");
        }
        if model.trim().is_empty() {
            bail!("尚未配置 AI 模型名称");
        }
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .connect_timeout(Duration::from_secs(15));
        builder = crate::redmine::apply_proxy(builder, proxy, base)?;
        Ok(Self {
            http: builder.build()?,
            endpoint: format!("{base}/chat/completions"),
            key: key.trim().to_string(),
            model: model.trim().to_string(),
            json_mode,
            temperature,
        })
    }

    pub async fn chat(&self, messages: &[Message], want_json: bool) -> Result<(String, Usage)> {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": self.temperature,
            "stream": false,
        });
        if want_json && self.json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }
        let mut req = self.http.post(&self.endpoint).json(&body);
        if !self.key.is_empty() {
            req = req.bearer_auth(&self.key);
        }
        let resp = req.send().await.map_err(|e| anyhow!("连接 AI 服务失败: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or_else(|| crate::redmine::truncate(&text, 200));
            return Err(match status.as_u16() {
                401 => crate::coded("AI_AUTH", format!("AI API Key 无效: {msg}")),
                402 => crate::coded("AI_BALANCE", format!("AI 账户余额不足: {msg}")),
                429 => crate::coded("AI_RATE_LIMIT", format!("AI 请求过于频繁，请稍后再试: {msg}")),
                _ => crate::coded("AI_HTTP", format!("AI 服务返回 HTTP {status}: {msg}")),
            });
        }
        #[derive(Deserialize)]
        struct Resp {
            choices: Vec<Choice>,
            #[serde(default)]
            usage: Usage,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: ChoiceMsg,
        }
        #[derive(Deserialize)]
        struct ChoiceMsg {
            #[serde(default)]
            content: Option<String>,
        }
        let r: Resp = serde_json::from_str(&text).context("AI 返回格式无法解析")?;
        let content = r
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| anyhow!("AI 未返回内容"))?;
        Ok((content, r.usage))
    }

    pub async fn chat_json<T: DeserializeOwned>(&self, messages: &[Message]) -> Result<(T, Usage)> {
        let (text, usage) = self.chat(messages, true).await?;
        let v = extract_json(&text).ok_or_else(|| anyhow!("AI 输出不是合法 JSON：{}", crate::redmine::truncate(&text, 200)))?;
        let parsed = serde_json::from_value(v).context("AI 输出的 JSON 结构不符合预期")?;
        Ok((parsed, usage))
    }
}

/// 兼容 ```json 代码块、前后带解释文字等情况
pub fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    let start = t.find(['{', '['])?;
    let end = t.rfind(['}', ']'])?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&t[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_fenced_json() {
        let v = extract_json("好的：\n```json\n{\"a\": 1}\n```").unwrap();
        assert_eq!(v["a"], 1);
    }
}
