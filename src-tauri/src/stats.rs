use serde::Serialize;

use crate::config::AppConfig;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub today_memories: u64,
    /// 本机已接入 agentmemory 的客户端数（检查各客户端配置文件中是否有接入痕迹）
    pub wired_clients: u64,
}

const PAGE_SIZE: &str = "200";
/// 分页上限：超过 2000 条最近记忆时停止翻页（防止极端情况下请求过多）
const MAX_PAGES: usize = 10;

/// 统计今日（本地时区 0 点起）新增的记忆数。
/// GET /agentmemory/memories 按 createdAt 倒序返回，逐页统计直到遇到昨日记忆。
pub async fn count_today(client: &reqwest::Client, config: &AppConfig) -> Result<Stats, String> {
    let today = chrono::Local::now().date_naive();
    let mut count: u64 = 0;
    let mut cursor: Option<String> = None;

    for _ in 0..MAX_PAGES {
        let mut req = client
            .get(format!("{}/agentmemory/memories", config.rest_url()))
            .bearer_auth(config.token.trim())
            .query(&[("limit", PAGE_SIZE)]);
        if let Some(c) = &cursor {
            req = req.query(&[("cursor", c)]);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| format!("响应解析失败: {e}"))?;

        let memories = resp["memories"]
            .as_array()
            .ok_or("响应缺少 memories 字段")?;
        if memories.is_empty() {
            break;
        }
        let mut all_older = true;
        for m in memories {
            let created = m["createdAt"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
            match created {
                Some(dt) if dt.with_timezone(&chrono::Local).date_naive() == today => {
                    count += 1;
                    all_older = false;
                }
                Some(_) => {
                    return Ok(Stats {
                        today_memories: count,
                        wired_clients: count_wired(),
                    })
                }
                None => continue,
            }
        }
        if all_older {
            break;
        }
        cursor = resp["nextCursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok(Stats {
        today_memories: count,
        wired_clients: count_wired(),
    })
}

/// 统计本机已接入 agentmemory 的客户端数（与接入页列表同一套检测逻辑）
pub fn count_wired() -> u64 {
    crate::agents::wired_count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::sync::Mutex;

    fn config_on(port: u16) -> AppConfig {
        AppConfig {
            rest_port: port,
            token: "t".into(),
            ..Default::default()
        }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap()
    }

    fn rfc3339(days_ago: i64) -> String {
        (chrono::Local::now() - chrono::Duration::days(days_ago)).to_rfc3339()
    }

    fn mem(days_ago: i64) -> serde_json::Value {
        serde_json::json!({"createdAt": rfc3339(days_ago), "id": format!("m{days_ago}")})
    }

    #[tokio::test]
    async fn counts_today_and_stops_at_yesterday() {
        let port = crate::testutil::spawn_server(move |_| {
            let body = serde_json::json!({
                "memories": [mem(0), mem(0), mem(1)],
                "nextCursor": null
            })
            .to_string();
            (200, body)
        });
        let stats = count_today(&client(), &config_on(port)).await.unwrap();
        assert_eq!(stats.today_memories, 2);
    }

    #[tokio::test]
    async fn paginates_until_older_memory() {
        let calls = std::sync::Arc::new(Mutex::new(0u32));
        let calls2 = calls.clone();
        let port = crate::testutil::spawn_server(move |path| {
            *calls2.lock().unwrap() += 1;
            if path.contains("cursor=p2") {
                (200, serde_json::json!({"memories": [mem(0), mem(2)], "nextCursor": null}).to_string())
            } else {
                (200, serde_json::json!({"memories": [mem(0), mem(0)], "nextCursor": "p2"}).to_string())
            }
        });
        let stats = count_today(&client(), &config_on(port)).await.unwrap();
        assert_eq!(stats.today_memories, 3);
        assert_eq!(*calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn empty_page_means_zero() {
        let port = crate::testutil::spawn_server(|_| {
            (200, serde_json::json!({"memories": [], "nextCursor": null}).to_string())
        });
        let stats = count_today(&client(), &config_on(port)).await.unwrap();
        assert_eq!(stats.today_memories, 0);
    }

    #[tokio::test]
    async fn missing_memories_field_is_error() {
        let port = crate::testutil::spawn_server(|_| (200, "{}".into()));
        let err = count_today(&client(), &config_on(port)).await.unwrap_err();
        assert!(err.contains("memories"), "unexpected: {err}");
    }
}
