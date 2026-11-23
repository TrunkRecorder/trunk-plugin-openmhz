//! OpenMHz — a Trunk Recorder Pro plugin that uploads recorded calls to
//! [OpenMHz](https://openmhz.com), as Trunk Recorder's OpenMHz uploader does.

mod upload;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use trunk_recorder_plugin::{
    Attempt, CallQueue, ConcludedCall, Host, Manifest, Plugin, QueueOptions, Setup, System, TalkgroupFilter, Talkgroups, format, topic,
};

use upload::{Upload, Uploader};

pub const DEFAULT_SERVER: &str = "https://api.openmhz.com";

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    /// Upload server
    ///
    /// Only for your own OpenMHz server
    #[schemars(url)]
    #[serde(alias = "uploadServer")]
    server: String,
    /// Shortest upload, s
    ///
    /// Shorter calls stay in your recordings · 0: every call with audio
    #[schemars(range(min = 0.0, max = 60.0))]
    min_upload_s: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config { server: DEFAULT_SERVER.into(), min_upload_s: 0.0 }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", default)]
struct SystemConfig {
    /// API key
    ///
    /// From the system's page on OpenMHz · empty: not uploaded
    #[schemars(extend("x-secret" = true, "x-required" = true))]
    api_key: String,
    /// Name on OpenMHz
    ///
    /// If not its short name here
    #[serde(alias = "openmhzSystemId")]
    system_name: String,
    // The talkgroups to upload: a menu, and the lists it asks for.
    #[serde(flatten)]
    talkgroups: Talkgroups,
}

/// Where a system's calls go.
struct Target {
    /// The queue's lane for its server: its host ("" for OpenMHz's own).
    lane: String,
    uploader: Arc<Uploader>,
    filter: TalkgroupFilter,
    /// The system, with its settings and its sites' ([`System::targets`]).
    system: System<Config, SystemConfig>,
}

/// The OpenMHz system settings send to: its name there (empty: `name`) and its API key (None: they have no key).
fn node<'a>(c: &'a SystemConfig, name: &'a str) -> Option<(&'a str, &'a str)> {
    let key = c.api_key.trim();
    let named = c.system_name.trim();
    (!key.is_empty()).then_some((if named.is_empty() { name } else { named }, key))
}

/// A call's audio, ms: `call_length` is whole seconds (a record without `call_length_ms`, its only length).
fn audio_ms(call: &ConcludedCall) -> i64 {
    Some(call.call.call_length_ms).filter(|&l| l > 0).unwrap_or((call.call.call_length * 1000.0).round() as i64)
}

/// One call's attempts, one for each OpenMHz system it went to, as one:
/// try again if any should be, else failed if any failed, else done if any took it.
fn together(attempts: Vec<Attempt>) -> Attempt {
    let rank = |a: &Attempt| match a {
        Attempt::Retry(_) => 3,
        Attempt::Fail(_) => 2,
        Attempt::Done { .. } => 1,
        Attempt::Skip(_) => 0,
    };
    attempts.into_iter().max_by_key(rank).unwrap_or(Attempt::Skip("nowhere to send it".into()))
}

struct OpenMhz {
    queue: CallQueue,
}

/// Uploads worked on at once, for each server. Every system sending to the
/// same server shares them: at 4800 calls an hour, the SDK's 2 would leave
/// each upload (its audio included) 1.5 s before calls start to wait.
const UPLOAD_THREADS: usize = 4;

/// A server's lane in the queue, and the name it goes by on the dashboard
/// ("OpenMHz at <host>"): OpenMHz's own is just "OpenMHz".
fn lane_of(server: &str) -> String {
    if server == DEFAULT_SERVER {
        return String::new();
    }
    server.split_once("://").map_or(server, |(_, r)| r).split('/').next().unwrap_or("").to_string()
}

impl Plugin for OpenMhz {
    type Config = Config;
    type SystemConfig = SystemConfig;

    fn manifest() -> Manifest {
        Manifest {
            name: "OpenMHz".into(),
            subscribe: vec![topic::CALL_CONCLUDED.into()],
            audio_formats: vec![format::M4A.into()],
            homepage: "https://openmhz.com".into(),
            // A system can upload to another server (one of its own), and set its own shortest call.
            system_overrides: vec!["server".into(), "minUploadS".into()],
            // A site that's an OpenMHz system of its own.
            site_overrides: vec!["apiKey".into(), "systemName".into()],
            ..trunk_recorder_plugin::manifest!()
        }
    }

    fn start(host: Host, setup: Setup<Config, SystemConfig>) -> Result<Self, String> {
        // OpenMHz takes M4A or MP3, not WAV.
        if !setup.has_format(format::M4A) {
            return Err("No M4A encoder: install ffmpeg, then Start again".into());
        }
        // One uploader for each server (a system can name a server of its own).
        let mut uploaders: HashMap<String, Arc<Uploader>> = HashMap::new();
        // By short name, and the name of each system id: a call finds its system by id, the same
        // after a rename (one saved by a recorder before ids, by short name).
        let mut targets: HashMap<String, Target> = HashMap::new();
        let mut names: HashMap<u32, String> = HashMap::new();
        let data_dir = setup.data_dir.clone();
        for s in setup.systems {
            let Some(c) = &s.config else { continue };
            let sites: Vec<(&str, (&str, &str))> = s.sites.iter().filter_map(|t| Some((t.name.as_str(), node(t.config.as_ref()?, &t.name)?))).collect();
            if node(c, &s.short_name).is_none() && sites.is_empty() {
                continue;
            }
            let server = s.settings.server.trim().trim_end_matches('/').to_string();
            if !(server.starts_with("https://") || server.starts_with("http://")) {
                return Err(format!("Upload server: a web address (https://…), not \"{server}\""));
            }
            let uploader = uploaders.entry(server.clone()).or_insert_with(|| Arc::new(Uploader::new(&server))).clone();
            let filter = c.talkgroups.filter_for(&s.short_name)?;
            let filtered = if filter.is_empty() { String::new() } else { format!(", talkgroups: {}", filter.describe()) };
            let lane = lane_of(&server);
            let to = if lane.is_empty() { String::new() } else { format!(" to {lane}") };
            match node(c, &s.short_name) {
                Some((name, key)) => host.info(format!("uploading {}{to} as {name} (key …{}){filtered}", s.short_name, last2(key))),
                None => host.info(format!("uploading {}{to} only from its sites with an OpenMHz system of their own{filtered}", s.short_name)),
            }
            for (site, (name, key)) in &sites {
                host.info(format!("uploading {}'s site {site} as {name} (key …{})", s.short_name, last2(key)));
            }
            if s.id != 0 {
                names.insert(s.id, s.short_name.clone());
            }
            targets.insert(s.short_name.clone(), Target { lane, uploader, filter, system: s });
        }
        if targets.is_empty() {
            return Err("No system has an API key: add one on its card in Setup".into());
        }
        // Each server is a lane of the queue: one that's down holds up no other.
        let targets = Arc::new(targets);
        let lanes = targets.clone();
        let opts = QueueOptions { threads: UPLOAD_THREADS, noun: "upload", endpoint: Some("OpenMHz".into()), ..QueueOptions::saved_in(&data_dir) };
        let names = Arc::new(names);
        // A call's system: by its id, else (a call from before ids) its short name.
        let of = move |call: &ConcludedCall| names.get(&call.call.system_id).cloned().unwrap_or_else(|| call.call.short_name.clone());
        let lane_of_call = of.clone();
        let lane = move |call: &ConcludedCall| lanes.get(&lane_of_call(call)).map(|t| t.lane.clone()).unwrap_or_default();
        let queue = CallQueue::start_by(host, opts, lane, move |call: &ConcludedCall| {
            let Some(t) = targets.get(&of(call)) else {
                return Attempt::Skip("no API key for this system".into());
            };
            if !t.filter.passes(call.call.talkgroup) {
                return Attempt::Skip(format!("talkgroup {} filtered out", call.call.talkgroup));
            }
            // An OpenMHz call is its audio: one with none (kept by "Keep calls with no audio"), or too little, stays local.
            let ms = audio_ms(call);
            if ms <= 0 {
                return Attempt::Skip("no audio".into());
            }
            let min_s = t.system.settings.min_upload_s;
            if (ms as f64) < min_s * 1000.0 {
                return Attempt::Skip(format!("shorter than {min_s} s"));
            }
            let Some(m4a) = &call.files.m4a else {
                return Attempt::Fail("no M4A for this call".into());
            };
            // Each OpenMHz system the sites that heard it send to, once (a site's named as the site unless given a name).
            let mut nodes: Vec<(&str, &str)> = Vec::new();
            for d in t.system.targets(&call.call) {
                let name = d.site.map_or(t.system.short_name.as_str(), |s| s.name.as_str());
                if let Some(n) = node(d.config, name).filter(|n| !nodes.contains(n)) {
                    nodes.push(n);
                }
            }
            if nodes.is_empty() {
                return Attempt::Skip("none of the sites that heard it has an OpenMHz system".into());
            }
            together(nodes.into_iter().map(|(system, api_key)| t.uploader.upload(&Upload { system, api_key, call: &call.call, audio: m4a })).collect())
        });
        Ok(OpenMhz { queue })
    }

    fn call_concluded(&mut self, call: ConcludedCall) {
        self.queue.push(call);
    }

    fn shutdown(&mut self, grace: Duration) {
        self.queue.shutdown(grace);
    }
}

/// The key's last two characters, to tell keys apart in the log.
fn last2(key: &str) -> String {
    let k: Vec<char> = key.trim().chars().collect();
    k[k.len().saturating_sub(2)..].iter().collect()
}

fn main() {
    trunk_recorder_plugin::run::<OpenMhz>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use trunk_recorder_plugin::testing::{self, MockServer};
    use trunk_recorder_plugin::{EXIT_CONFIG, HostMessage, Outcome, SiteInfo, SiteRef, State, SystemInfo};

    /// OpenMHz as its upload handler answers: key "good" is right for "sys1".
    fn openmhz() -> MockServer {
        MockServer::start(|req| {
            let key = String::from_utf8(req.form_field("api_key").unwrap_or_default()).unwrap();
            match (req.path.as_str(), key.as_str()) {
                ("/sys1/upload", "good") => (200, String::new()),
                ("/sys1/upload", _) => (500, "API Keys do not match!\n".into()),
                (_, _) => (500, "ShortName does not exist: x\n".into()),
            }
        })
    }

    /// A site with an OpenMHz system of its own (its settings in Setup): the calls it heard go there
    /// (named as the site unless given a name), whichever copy was kept; those only the others heard, to the system's.
    #[test]
    fn a_site_of_its_own() {
        let (dir, server) = (testing::temp_dir("omhz"), openmhz());
        // (The recorder sends each site's settings as the system's with what it sets again.)
        let mut h = testing::hello(&dir, json!({ "server": server.url() }));
        h.systems[0].config = json!({ "apiKey": "good" });
        h.systems[0].sites = vec![
            SiteInfo { id: 7, name: "north".into(), config: Value::Null },
            SiteInfo { id: 8, name: "south".into(), config: json!({ "apiKey": "southkey" }) },
            SiteInfo { id: 9, name: "east".into(), config: json!({ "apiKey": "eastkey", "systemName": "east-omhz" }) },
        ];
        let call = |tg, heard: &[(u32, &str)]| {
            let mut c = testing::call(&dir, "sys1", tg);
            c.call.site = heard[0].1.into();
            c.call.sites = heard.iter().map(|&(id, name)| SiteRef { id, name: name.into() }).collect();
            HostMessage::CallConcluded(c)
        };
        testing::run::<OpenMhz>([HostMessage::Hello(h), call(101, &[(7, "north")]), call(102, &[(7, "north"), (8, "south")]), call(103, &[(9, "east")])]);
        let mut sent: Vec<(String, String)> =
            server.requests().iter().map(|r| (r.path.clone(), String::from_utf8(r.form_field("api_key").unwrap_or_default()).unwrap())).collect();
        sent.sort();
        assert_eq!(
            sent,
            [
                ("/east-omhz/upload".into(), "eastkey".into()),
                ("/south/upload".into(), "southkey".into()),
                ("/sys1/upload".into(), "good".into()),
                ("/sys1/upload".into(), "good".into())
            ],
            "north's: the system's; north and south's: both; east's: its own, by its name there"
        );
    }

    fn hello(dir: &std::path::Path, server: &str, system: Value) -> HostMessage {
        let mut h = testing::hello(dir, json!({ "server": server }));
        h.systems[0].config = system;
        HostMessage::Hello(h)
    }

    #[test]
    fn a_call_with_no_audio_is_skipped_not_uploaded() {
        let (dir, server) = (testing::temp_dir("openmhz-silent"), openmhz());
        let mut call = testing::call(&dir, "sys1", 101);
        (call.call.call_length, call.call.call_length_ms) = (0.0, 0);
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "good" })), HostMessage::CallConcluded(call)]);
        let r = out.results();
        assert_eq!(r[0].1, Outcome::Skipped, "{r:?}");
        assert!(r[0].2.contains("no audio"), "{}", r[0].2);
        assert!(server.requests().is_empty());
    }

    /// The recorder's `call_length` is whole seconds: a 0.4 s call says 0 there, and has audio.
    fn call_of(dir: &std::path::Path, tg: u32, ms: i64) -> HostMessage {
        let mut c = testing::call(dir, "sys1", tg);
        (c.call.call_length, c.call.call_length_ms) = ((ms as f64 / 1000.0).round(), ms);
        HostMessage::CallConcluded(c)
    }

    #[test]
    fn by_default_any_call_with_audio_is_uploaded() {
        let (dir, server) = (testing::temp_dir("openmhz-short"), openmhz());
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "good" })), call_of(&dir, 101, 400)]);
        assert_eq!(out.results()[0].1, Outcome::Ok);
    }

    #[test]
    fn min_upload_s_keeps_short_calls_local() {
        let (dir, server) = (testing::temp_dir("openmhz-min"), openmhz());
        let mut h = testing::hello(&dir, json!({ "server": server.url(), "minUploadS": 1.0 }));
        h.systems[0].config = json!({ "apiKey": "good" });
        let calls = [(101, 400), (102, 990), (103, 1000), (104, 5000)].map(|(tg, ms)| call_of(&dir, tg, ms));
        let out = testing::run::<OpenMhz>(std::iter::once(HostMessage::Hello(h)).chain(calls));
        // (Uploads run at once: results come as they finish.)
        let mut results = out.results();
        results.sort_by(|a, b| a.0.cmp(&b.0));
        let outcomes: Vec<Outcome> = results.into_iter().map(|r| r.1).collect();
        assert_eq!(outcomes, [Outcome::Skipped, Outcome::Skipped, Outcome::Ok, Outcome::Ok]);
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn a_patched_call_sends_its_patch_list() {
        let (dir, server) = (testing::temp_dir("openmhz-patch"), openmhz());
        let mut call = testing::call(&dir, "sys1", 65001);
        call.call.patched_talkgroups = vec![101, 202, 65001];
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "good" })), HostMessage::CallConcluded(call)]);
        assert!(out.ready(), "{:?}", out.messages);
        let r = &server.requests()[0];
        assert_eq!(String::from_utf8(r.form_field("patch_list").unwrap()).unwrap(), "[101,202,65001]");
    }

    #[test]
    fn uploads_a_call_as_trunk_recorder_does() {
        let (dir, server) = (testing::temp_dir("openmhz"), openmhz());
        let call = testing::call(&dir, "sys1", 101);
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "good" })), HostMessage::CallConcluded(call.clone())]);
        assert!(out.ready(), "{:?}", out.messages);
        assert_eq!(out.results(), vec![(call.path.clone(), Outcome::Ok, String::new(), String::new())]);
        let r = &server.requests()[0];
        assert_eq!((r.method.as_str(), r.path.as_str()), ("POST", "/sys1/upload"));
        let field = |n: &str| String::from_utf8(r.form_field(n).unwrap()).unwrap();
        assert_eq!(field("talkgroup_num"), "101");
        assert_eq!(field("freq"), "851012500");
        assert_eq!(field("start_time"), call.call.start_time.to_string());
        assert_eq!(field("stop_time"), call.call.stop_time.to_string());
        assert_eq!(field("call_length"), "3");
        assert_eq!(field("error_count"), "2");
        assert_eq!(field("emergency"), "0");
        assert_eq!(field("patch_list"), "[]");
        let sources: Value = serde_json::from_str(&field("source_list")).unwrap();
        assert_eq!(sources, json!([{ "pos": 0.0, "src": 1234, "tag": "" }]));
        // The M4A, by a name OpenMHz accepts.
        assert!(r.form_file_name("call").unwrap().ends_with(".m4a"));
        assert_eq!(r.form_field("call").unwrap(), std::fs::read(call.files.m4a.unwrap()).unwrap());
    }

    #[test]
    fn a_wrong_key_fails_without_retrying() {
        let (dir, server) = (testing::temp_dir("openmhz"), openmhz());
        let call = testing::call(&dir, "sys1", 101);
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "bad" })), HostMessage::CallConcluded(call)]);
        let r = out.results();
        assert_eq!(r[0].1, Outcome::Failed);
        assert!(r[0].2.contains("API key"), "{}", r[0].2);
        assert_eq!(server.requests().len(), 1);
    }

    /// A call saved in an earlier run carries that run's system number; it
    /// is still uploaded with its own system's key, found by short name.
    #[test]
    fn a_call_from_an_earlier_run_finds_its_system_by_name() {
        let (dir, server) = (testing::temp_dir("openmhz"), openmhz());
        let mut call = testing::call(&dir, "sys1", 5);
        call.system = 7;
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "good" })), HostMessage::CallConcluded(call)]);
        assert_eq!(out.results()[0].1, Outcome::Ok);
        assert_eq!(server.requests()[0].path, "/sys1/upload");
    }

    #[test]
    fn a_system_by_another_name() {
        let (dir, server) = (testing::temp_dir("openmhz"), openmhz());
        let out = testing::run::<OpenMhz>([
            hello(&dir, server.url(), json!({ "apiKey": "good", "systemName": "elsewhere" })),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 5)),
        ]);
        assert_eq!(server.requests()[0].path, "/elsewhere/upload");
        assert!(out.results()[0].2.contains("no system named elsewhere"));
    }

    #[test]
    fn trunk_recorders_setting_name_still_works() {
        let (dir, server) = (testing::temp_dir("openmhz"), openmhz());
        testing::run::<OpenMhz>([
            hello(&dir, server.url(), json!({ "apiKey": "good", "openmhzSystemId": "old" })),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 5)),
        ]);
        assert_eq!(server.requests()[0].path, "/old/upload");
    }

    #[test]
    fn a_server_that_is_down_keeps_the_call_for_next_time() {
        let dir = testing::temp_dir("openmhz");
        // Nothing listens on port 9 (discard) on a test machine.
        let out = testing::run::<OpenMhz>([
            hello(&dir, "http://127.0.0.1:9", json!({ "apiKey": "good" })),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 5)),
        ]);
        assert!(out.results().is_empty(), "{:?}", out.results());
        assert!(matches!(out.status(), Some((State::Warning, _))));
        let saved = std::fs::read_to_string(dir.join("data/queue.jsonl")).unwrap();
        assert_eq!(saved.lines().count(), 1);
    }

    #[test]
    fn trunk_recorders_server_setting_name_works() {
        let (dir, server) = (testing::temp_dir("openmhz"), openmhz());
        let mut h = testing::hello(&dir, json!({ "uploadServer": server.url() }));
        h.systems[0].config = json!({ "apiKey": "good" });
        testing::run::<OpenMhz>([HostMessage::Hello(h), HostMessage::CallConcluded(testing::call(&dir, "sys1", 5))]);
        assert_eq!(server.requests().len(), 1);
    }

    /// Only some talkgroups, or all but some: the others are skipped, and an entry that isn't a talkgroup stops it starting.
    #[test]
    fn talkgroups() {
        let (dir, server) = (testing::temp_dir("openmhz"), openmhz());
        let out = testing::run::<OpenMhz>([
            hello(&dir, server.url(), json!({ "apiKey": "good", "talkgroupMode": "only", "talkgroupAllow": ["100-199", "5*"], "talkgroupDeny": [150] })),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 101)),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 150)),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 512)),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 300)),
        ]);
        let mut sent: Vec<String> = server.requests().iter().map(|r| String::from_utf8(r.form_field("talkgroup_num").unwrap()).unwrap()).collect();
        sent.sort();
        assert_eq!(sent, ["101", "512"]);
        let skipped: Vec<_> = out.results().into_iter().filter(|r| r.1 == Outcome::Skipped).collect();
        assert_eq!(skipped.len(), 2, "{skipped:?}");
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "good", "talkgroupMode": "allBut", "talkgroupDeny": ["1-"] }))]);
        assert_eq!(out.exit_code, EXIT_CONFIG);
        assert!(out.status().unwrap().1.starts_with("sys1's talkgroups: \"1-\" isn't a talkgroup"), "{:?}", out.status());
        // The form: the menu after the name on OpenMHz, its lists when they apply.
        let m = trunk_recorder_plugin::describe::<OpenMhz>();
        let s = m.system_config.unwrap();
        assert_eq!(s["x-order"], json!(["apiKey", "systemName", "talkgroupMode", "talkgroupAllow", "talkgroupDeny"]));
        // What a system can set again is one of its settings.
        assert!(m.system_overrides.iter().all(|k| m.config.as_ref().unwrap()["properties"].get(k).is_some()), "{:?}", m.system_overrides);
    }

    #[test]
    fn needs_a_key() {
        let dir = testing::temp_dir("openmhz");
        let out = testing::run::<OpenMhz>([hello(&dir, DEFAULT_SERVER, Value::Null)]);
        assert_eq!(out.exit_code, EXIT_CONFIG);
        assert!(out.status().unwrap().1.contains("API key"));
    }

    #[test]
    fn needs_m4a() {
        let dir = testing::temp_dir("openmhz");
        let mut h = testing::hello(&dir, Value::Null);
        h.systems[0].config = json!({ "apiKey": "good" });
        h.audio_formats = vec!["wav".into()];
        let out = testing::run::<OpenMhz>([HostMessage::Hello(h)]);
        assert_eq!(out.exit_code, EXIT_CONFIG);
        assert!(out.status().unwrap().1.contains("ffmpeg"));
    }

    #[test]
    fn systems_without_a_key_are_skipped() {
        let (dir, server) = (testing::temp_dir("openmhz"), openmhz());
        let call = testing::call(&dir, "sys2", 5);
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "good" })), HostMessage::CallConcluded(call)]);
        assert_eq!(out.results()[0].1, Outcome::Skipped);
        assert!(server.requests().is_empty());
    }

    /// A system can send its calls to a server of its own, with an endpoint
    /// status of its own; OpenMHz's own server is just "OpenMHz".
    #[test]
    fn a_system_sets_its_own_shortest_upload() {
        let (dir, server) = (testing::temp_dir("openmhz-min-own"), openmhz());
        let mut h = testing::hello(&dir, json!({ "server": server.url() }));
        h.systems[0].config = json!({ "apiKey": "good" });
        h.systems.push(SystemInfo {
            index: 1,
            short_name: "sys2".into(),
            kind: "p25".into(),
            config: json!({ "apiKey": "good", "systemName": "sys1" }),
            settings: json!({ "minUploadS": 2.0 }),
            ..Default::default()
        });
        let out = testing::run::<OpenMhz>([
            HostMessage::Hello(h),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 101)),
            HostMessage::CallConcluded({
                let mut c = testing::call(&dir, "sys2", 202);
                c.call.call_length_ms = 1500;
                c
            }),
        ]);
        let mut results = out.results();
        results.sort_by(|a, b| a.0.cmp(&b.0));
        let outcomes: Vec<Outcome> = results.into_iter().map(|r| r.1).collect();
        assert_eq!(outcomes, [Outcome::Ok, Outcome::Skipped]);
    }

    #[test]
    fn a_system_overrides_the_server() {
        let (dir, main, own) = (testing::temp_dir("openmhz"), openmhz(), openmhz());
        let mut h = testing::hello(&dir, json!({ "server": main.url() }));
        h.systems[0].config = json!({ "apiKey": "good" });
        h.systems.push(SystemInfo {
            index: 1,
            short_name: "sys2".into(),
            kind: "p25".into(),
            config: json!({ "apiKey": "good", "systemName": "sys1" }),
            settings: json!({ "server": own.url() }),
            ..Default::default()
        });
        let out = testing::run::<OpenMhz>([
            HostMessage::Hello(h),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 101)),
            HostMessage::CallConcluded(testing::call(&dir, "sys2", 202)),
        ]);
        assert!(out.results().iter().all(|r| r.1 == Outcome::Ok), "{:?}", out.results());
        assert_eq!((main.requests().len(), own.requests().len()), (1, 1));
        let mut names: Vec<String> = out.metrics().last().unwrap().endpoints.iter().map(|e| e.name.clone()).collect();
        let mut want = [main.url(), own.url()].map(|u| format!("OpenMHz at {}", lane_of(u)));
        names.sort();
        want.sort();
        assert_eq!(names, want);
        assert_eq!(lane_of(DEFAULT_SERVER), "");
    }
}
