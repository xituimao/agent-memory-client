use serde::Serialize;

use crate::config::AppConfig;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub today_memories: u64,
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
                Some(_) => return Ok(Stats { today_memories: count }),
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
    Ok(Stats { today_memories: count })
}
