//! Mirakurun のチャンネル一覧の取得とチューニング空間の構築。
//!
//! 元実装の InitChannel / GetApiChannels / EnumTuningSpace / EnumChannelName /
//! SetChannel のチャンネル解決部分に相当する。

use crate::config::Config;
use crate::http;
use crate::util::SPACE_NUM;
use serde_json::Value;

/// フラットなチャンネル(またはサービス)1件分。
#[derive(Debug, Clone)]
pub struct ChannelEntry {
    pub type_: String,
    pub channel: String,
    pub name: String,
    pub service_id: Option<u64>,
    /// このチャンネルで X-Mirakc-Tuner に指定するチューナー名。
    /// API 応答に `tuner` フィールドがあればそれを使う(無ければ空 → 設定値)。
    pub tuner: String,
}

/// チャンネル一覧と、そこから導出したチューニング空間情報。
#[derive(Debug, Default, Clone)]
pub struct Channels {
    /// API から取得したフラットなチャンネル列。
    entries: Vec<ChannelEntry>,
    /// チューニング空間ごとの type 名(g_pType 相当)。
    types: Vec<String>,
    /// 各チューニング空間の先頭インデックス(g_Channel_Base 相当)。
    bases: Vec<usize>,
}

impl Channels {
    /// チャンネル情報が空(取得失敗)かどうか。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 最大のチューニング空間インデックス(g_Max_Type 相当)。
    fn max_type(&self) -> usize {
        self.types.len().saturating_sub(1)
    }

    /// Mirakurun API からチャンネル一覧を取得して構築する。
    pub fn fetch(cfg: &Config) -> Channels {
        let path = if cfg.service_split == 1 {
            "/api/services"
        } else {
            "/api/channels"
        };

        let body = match get_body(cfg, path) {
            Some(b) => b,
            None => return Channels::default(),
        };

        // チューナー固定時(GLOBAL.TUNER)は、そのチューナーが対応する放送種別
        // のみを一覧に残す。衛星専用チューナーに地上波が、地上波専用チューナーに
        // 衛星が表示されて選局時に 404 になる問題を防ぐ。
        let allowed = tuner_channel_types(cfg);
        Channels::from_json_filtered(&body, cfg.service_split == 1, allowed.as_deref())
    }

    /// JSON 文字列からチャンネル一覧を構築する。`allowed_types` が Some の場合は
    /// その種別に一致するチャンネルのみを残す(チューナー固定時の絞り込み)。
    pub fn from_json_filtered(
        body: &str,
        service_split: bool,
        allowed_types: Option<&[String]>,
    ) -> Channels {
        let value: Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => return Channels::default(),
        };

        let array = match value.as_array() {
            Some(a) if !a.is_empty() => a,
            _ => return Channels::default(),
        };

        let mut entries = Vec::with_capacity(array.len());
        for item in array {
            let entry = if service_split {
                let ch = &item["channel"];
                ChannelEntry {
                    type_: ch["type"].as_str().unwrap_or("").to_string(),
                    channel: ch["channel"].as_str().unwrap_or("").to_string(),
                    name: item["name"].as_str().unwrap_or("").to_string(),
                    // SERVICE_SPLIT=1 のストリーム URL は Mirakurun のサービス id
                    // (NID*100000+SID)を使う。BonDriver_mirakc と同じ。
                    service_id: item["id"].as_u64(),
                    tuner: ch["tuner"].as_str().unwrap_or("").to_string(),
                }
            } else {
                ChannelEntry {
                    type_: item["type"].as_str().unwrap_or("").to_string(),
                    channel: item["channel"].as_str().unwrap_or("").to_string(),
                    name: item["name"].as_str().unwrap_or("").to_string(),
                    service_id: None,
                    tuner: item["tuner"].as_str().unwrap_or("").to_string(),
                }
            };
            // チューナー固定時は対応種別のみ残す(大文字小文字は無視)。
            if let Some(allowed) = allowed_types {
                let t = entry.type_.to_ascii_uppercase();
                if !allowed.iter().any(|a| a.eq_ignore_ascii_case(&t)) {
                    continue;
                }
            }
            entries.push(entry);
        }

        let (types, bases) = build_spaces(&entries);
        Channels {
            entries,
            types,
            bases,
        }
    }

    /// 使用可能なチューニング空間名を返す(EnumTuningSpace 相当)。
    pub fn enum_tuning_space(&self, space: u32) -> Option<&str> {
        self.types.get(space as usize).map(|s| s.as_str())
    }

    /// 指定空間・チャンネルの名前を返す(EnumChannelName 相当)。
    pub fn enum_channel_name(&self, space: u32, channel: u32) -> Option<&str> {
        let idx = self.resolve_index(space, channel)?;
        self.entries.get(idx).map(|e| e.name.as_str())
    }

    /// 指定空間・チャンネルのエントリを返す(SetChannel 用)。
    pub fn entry(&self, space: u32, channel: u32) -> Option<&ChannelEntry> {
        let idx = self.resolve_index(space, channel)?;
        self.entries.get(idx)
    }

    /// (空間, 相対チャンネル) からフラットインデックスを解決する。
    fn resolve_index(&self, space: u32, channel: u32) -> Option<usize> {
        let space = space as usize;
        if space >= self.types.len() {
            return None;
        }
        // 最後の空間以外は、相対チャンネルが空間の幅を超えていないか確認する。
        if space < self.max_type() {
            let width = self.bases[space + 1] - self.bases[space];
            if channel as usize >= width {
                return None;
            }
        }
        let idx = channel as usize + self.bases[space];
        if idx >= self.entries.len() {
            None
        } else {
            Some(idx)
        }
    }
}

/// 設定に応じたトランスポートで API を GET し、ボディを返す。
/// SERVER_TYPE="unix" なら Unix ドメインソケット、それ以外は TCP。
fn get_body(cfg: &Config, path: &str) -> Option<String> {
    let mut stream = crate::transport::connect(cfg)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    http::get_via(&mut stream, &cfg.server_host, path, cfg.priority)
}

/// 固定チューナー(GLOBAL.TUNER)が対応する放送種別を `/api/tuners` から取得する。
///
/// 戻り値:
/// - `Some(types)`: チューナーが見つかり、対応種別で絞り込む。
/// - `None`: チューナー未指定、または `/api/tuners` 取得/解析失敗。
///   この場合は絞り込みを行わない(従来動作を維持し、全波を表示)。
fn tuner_channel_types(cfg: &Config) -> Option<Vec<String>> {
    if cfg.tuner.is_empty() {
        return None;
    }
    let body = get_body(cfg, "/api/tuners")?;
    let types = tuner_types_from_json(&body, &cfg.tuner)?;
    // 対応種別が空(取得できず)の場合は絞り込まない(全波を維持)。
    if types.is_empty() { None } else { Some(types) }
}

/// `/api/tuners` の JSON から、指定チューナー名の対応種別を取り出す。
///
/// 応答は Mirakurun 互換のチューナー配列で、各要素は `name` と `types`
/// (GR/BS/CS/SKY/BS4K/CATV)を持つ。1件だけの応答(オブジェクト)にも対応する。
pub fn tuner_types_from_json(body: &str, name: &str) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(body).ok()?;
    // 通常は配列。単一オブジェクトで返すサーバにも一応対応する。
    let tuners: Vec<&Value> = match &value {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![&value],
        _ => return None,
    };
    for tuner in tuners {
        if tuner["name"].as_str() == Some(name) {
            let types: Vec<String> = tuner["types"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            return Some(types);
        }
    }
    None
}

/// フラットなチャンネル列を type ごとにグループ化し、
/// チューニング空間の type 名と先頭インデックスを構築する。
///
/// 元実装と同様、エントリは type ごとにまとまっている前提で、
/// type が切り替わった位置を空間の境界とみなす。空間数は SPACE_NUM-1 まで。
fn build_spaces(entries: &[ChannelEntry]) -> (Vec<String>, Vec<usize>) {
    let mut types: Vec<String> = Vec::new();
    let mut bases: Vec<usize> = Vec::new();

    for (i, e) in entries.iter().enumerate() {
        match types.last() {
            None => {
                types.push(e.type_.clone());
                bases.push(0);
            }
            Some(last) if *last != e.type_ => {
                if types.len() >= SPACE_NUM - 1 {
                    break;
                }
                types.push(e.type_.clone());
                bases.push(i);
            }
            _ => {}
        }
    }

    (types, bases)
}
