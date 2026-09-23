//! 把环境变量映射到配置字段。
//!
//! 约定：前缀 `DCU_` + 配置项路径的大写下划线形式，
//! 例如 `compose.max_depth` 对应 `DCU_COMPOSE_MAX_DEPTH`；
//! 列表用逗号分隔（`DCU_WHITELIST_0` 这类写法不支持）。

use std::env;

/// 带作用域的配置路径，负责拼接环境变量名。
pub struct Scope {
    prefix: String,
}

impl Scope {
    pub fn new() -> Self {
        Self {
            prefix: "DCU_".to_string(),
        }
    }

    pub fn with(&self, key: &str) -> Scope {
        Scope {
            prefix: format!("{}{}_", self.prefix, key.to_uppercase()),
        }
    }

    pub fn name(&self, key: &str) -> String {
        format!("{}{}", self.prefix, key.to_uppercase())
    }

    /// 取布尔值；变量不存在或无法解析时返回 `None`，由调用方决定默认值。
    pub fn bool(&self, key: &str) -> Option<bool> {
        let raw = self.raw(key)?;
        match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "y" | "on" => Some(true),
            "0" | "false" | "no" | "n" | "off" => Some(false),
            _ => None,
        }
    }

    pub fn usize(&self, key: &str) -> Option<usize> {
        self.raw(key)?.trim().parse().ok()
    }

    pub fn string(&self, key: &str) -> Option<String> {
        let raw = self.raw(key)?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// 逗号分隔的列表，空白项会被丢弃。
    pub fn list(&self, key: &str) -> Option<Vec<String>> {
        let raw = self.raw(key)?;
        let items: Vec<String> = raw
            .split(',')
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect();
        if items.is_empty() {
            None
        } else {
            Some(items)
        }
    }

    fn raw(&self, key: &str) -> Option<String> {
        env::var(self.name(key)).ok()
    }
}
