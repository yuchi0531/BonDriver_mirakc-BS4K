//! プラットフォーム非依存のロジックに対するユニットテスト。
//! `cargo test`(Linux)で実行できる。

use crate::channels::Channels;
use crate::config::Config;

#[test]
fn config_parse_basic_and_quotes() {
    let cfg = Config::parse(
        r#"
[GLOBAL]
SERVER_HOST="192.168.1.10"
SERVER_PORT=40772
DECODE_B25=1
PRIORITY=10
SERVICE_SPLIT=0
TUNER="tuner0"
"#,
    );
    assert_eq!(cfg.server_host, "192.168.1.10");
    assert_eq!(cfg.server_port, "40772");
    assert_eq!(cfg.port_num(), 40772);
    // 既定ポートは BonDriver_mirakc に合わせて 40772。
    assert_eq!(Config::default().port_num(), 40772);
    assert_eq!(cfg.decode_b25, 1);
    assert_eq!(cfg.priority, 10);
    assert_eq!(cfg.service_split, 0);
    assert_eq!(cfg.tuner, "tuner0");
    assert!(!cfg.use_unix_socket());
}

#[test]
fn config_defaults_when_empty() {
    let cfg = Config::parse("");
    assert_eq!(cfg.server_host, "localhost");
    assert_eq!(cfg.port_num(), 40772);
    assert_eq!(cfg.server_type, "http");
    assert!(cfg.tuner.is_empty());
}

#[test]
fn config_unix_socket() {
    let cfg = Config::parse(
        r#"
[GLOBAL]
SERVER_TYPE="unix"
SERVER_SOCKPATH="/var/run/mirakc.sock"
"#,
    );
    assert!(cfg.use_unix_socket());
    assert_eq!(cfg.server_sockpath, "/var/run/mirakc.sock");
}

#[test]
fn config_is_case_insensitive() {
    let cfg = Config::parse("[global]\nserver_host=\"h\"\nserver_port=1234\n");
    assert_eq!(cfg.server_host, "h");
    assert_eq!(cfg.port_num(), 1234);
}

#[test]
fn config_leading_int_semantics() {
    // GetPrivateProfileInt 互換: 先頭の整数のみ解釈する。
    let cfg = Config::parse("[GLOBAL]\nPRIORITY=42abc\n");
    assert_eq!(cfg.priority, 42);
}

#[test]
fn channels_parse_and_spaces() {
    // mirakc の /api/channels 応答(GR/BS/BS4K)。BS4K は不透明な channel 文字列。
    let json = r#"[
        {"type":"GR","channel":"27","name":"TOKYO MX","services":[]},
        {"type":"BS","channel":"BS01","name":"BS1","services":[]},
        {"type":"BS4K","channel":"45328","name":"BS4K-test","services":[]},
        {"type":"BS4K","channel":"0xB110","name":"BS4K-two","services":[]}
    ]"#;
    let ch = Channels::from_json_filtered(json, false, None);
    assert_eq!(ch.enum_tuning_space(0), Some("GR"));
    assert_eq!(ch.enum_tuning_space(1), Some("BS"));
    assert_eq!(ch.enum_tuning_space(2), Some("BS4K"));
    assert_eq!(ch.enum_tuning_space(3), None);

    assert_eq!(ch.enum_channel_name(0, 0), Some("TOKYO MX"));
    assert_eq!(ch.enum_channel_name(1, 0), Some("BS1"));
    assert_eq!(ch.enum_channel_name(2, 0), Some("BS4K-test"));
    assert_eq!(ch.enum_channel_name(2, 1), Some("BS4K-two"));

    // 空間幅を超える相対チャンネルは None。
    assert_eq!(ch.enum_channel_name(0, 1), None);

    // BS4K チャンネル文字列はそのまま保持される(StreamID を透過)。
    let e = ch.entry(2, 0).unwrap();
    assert_eq!(e.type_, "BS4K");
    assert_eq!(e.channel, "45328");
    let e2 = ch.entry(2, 1).unwrap();
    assert_eq!(e2.channel, "0xB110");
}

#[test]
fn channels_service_split_uses_channel_object() {
    // /api/services 応答。name はサービス名、type/channel は入れ子の channel から取る。
    // ストリーム URL は Mirakurun のサービス id (`id`) を使う。
    let json = r#"[
        {"id":300101,"serviceId":101,"name":"4K-svc",
         "channel":{"type":"BS4K","channel":"45328"}}
    ]"#;
    let ch = Channels::from_json_filtered(json, true, None);
    assert_eq!(ch.enum_tuning_space(0), Some("BS4K"));
    let e = ch.entry(0, 0).unwrap();
    assert_eq!(e.name, "4K-svc");
    assert_eq!(e.channel, "45328");
    assert_eq!(e.service_id, Some(300101));
}

#[test]
fn channels_per_channel_tuner_is_parsed() {
    let json = r#"[
        {"type":"BS4K","channel":"45328","name":"4K","tuner":"tuner1","services":[]}
    ]"#;
    let ch = Channels::from_json_filtered(json, false, None);
    assert_eq!(ch.entry(0, 0).unwrap().tuner, "tuner1");
}

#[test]
fn channels_invalid_json_is_empty() {
    let ch = Channels::from_json_filtered("not json", false, None);
    assert!(ch.is_empty());
    assert_eq!(ch.enum_tuning_space(0), None);
}

#[test]
fn tuner_types_from_api_response() {
    // mirakc の /api/tuners 応答。チューナー名 -> 対応種別。
    let json = r#"[
        {"index":0,"name":"sat","types":["BS","CS","SKY","BS4K"]},
        {"index":1,"name":"gr","types":["GR"]},
        {"index":2,"name":"catv","types":["CATV"]}
    ]"#;
    assert_eq!(
        crate::channels::tuner_types_from_json(json, "sat"),
        Some(vec![
            "BS".to_string(),
            "CS".to_string(),
            "SKY".to_string(),
            "BS4K".to_string()
        ])
    );
    assert_eq!(
        crate::channels::tuner_types_from_json(json, "gr"),
        Some(vec!["GR".to_string()])
    );
    // 存在しないチューナー名は None。
    assert_eq!(crate::channels::tuner_types_from_json(json, "nope"), None);
}

#[test]
fn channel_filter_by_tuner_types_satellite_only() {
    // 衛星専用チューナー(BS/CS/SKY/BS4K)を固定した場合、地上波(GR)は出ない。
    let json = r#"[
        {"type":"GR","channel":"27","name":"TOKYO MX","services":[]},
        {"type":"BS","channel":"BS01","name":"BS1","services":[]},
        {"type":"BS4K","channel":"45328","name":"BS4K-test","services":[]}
    ]"#;
    let allowed = vec![
        "BS".to_string(),
        "CS".to_string(),
        "SKY".to_string(),
        "BS4K".to_string(),
    ];
    let ch = Channels::from_json_filtered(json, false, Some(&allowed));
    // GR 空間は消え、BS / BS4K のみ。
    assert_eq!(ch.enum_tuning_space(0), Some("BS"));
    assert_eq!(ch.enum_tuning_space(1), Some("BS4K"));
    assert_eq!(ch.enum_tuning_space(2), None);
    // space0=BS の先頭は BS1。
    assert_eq!(ch.enum_channel_name(0, 0), Some("BS1"));
}

#[test]
fn channel_filter_by_tuner_types_ground_only() {
    // 地上波専用チューナー(GR)を固定した場合、衛星は出ない。
    let json = r#"[
        {"type":"GR","channel":"27","name":"TOKYO MX","services":[]},
        {"type":"BS","channel":"BS01","name":"BS1","services":[]}
    ]"#;
    let allowed = vec!["GR".to_string()];
    let ch = Channels::from_json_filtered(json, false, Some(&allowed));
    assert_eq!(ch.enum_tuning_space(0), Some("GR"));
    assert_eq!(ch.enum_tuning_space(1), None);
}

#[test]
fn channel_filter_is_case_insensitive() {
    // 種別の大小文字差を吸収する(設定は小文字でも可)。
    let json = r#"[{"type":"BS","channel":"BS01","name":"BS1","services":[]}]"#;
    let allowed = vec!["bs".to_string()];
    let ch = Channels::from_json_filtered(json, false, Some(&allowed));
    assert_eq!(ch.enum_tuning_space(0), Some("BS"));
}

#[test]
fn no_filter_keeps_all_types() {
    let json = r#"[
        {"type":"GR","channel":"27","name":"TOKYO MX","services":[]},
        {"type":"BS","channel":"BS01","name":"BS1","services":[]}
    ]"#;
    let ch = Channels::from_json_filtered(json, false, None);
    assert_eq!(ch.enum_tuning_space(0), Some("GR"));
    assert_eq!(ch.enum_tuning_space(1), Some("BS"));
}

#[test]
fn to_wide_is_nul_terminated_utf16() {
    let w = crate::util::to_wide("Aあ");
    assert_eq!(w, vec![0x0041, 0x3042, 0x0000]);
}

#[test]
fn parse_status_code_variants() {
    assert_eq!(
        crate::tuner::parse_status_code(b"HTTP/1.1 200 OK"),
        Some(200)
    );
    assert_eq!(
        crate::tuner::parse_status_code(b"HTTP/1.0 503 Service Unavailable\r\nX: y"),
        Some(503)
    );
    assert_eq!(
        crate::tuner::parse_status_code(b"HTTP/1.1 404 Not Found\r\n"),
        Some(404)
    );
    assert_eq!(crate::tuner::parse_status_code(b"garbage"), None);
}

#[test]
fn header_value_sanitization() {
    // 正常値はそのまま(前後空白は除去)。
    assert_eq!(
        crate::tuner::sanitize_header_value(" tuner0 "),
        Some("tuner0".to_string())
    );
    // 空・CRLF 注入は拒否。
    assert_eq!(crate::tuner::sanitize_header_value(""), None);
    assert_eq!(crate::tuner::sanitize_header_value("a\r\nX-Evil: 1"), None);
}
