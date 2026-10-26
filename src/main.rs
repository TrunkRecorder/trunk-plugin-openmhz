//! OpenMHz — a Trunk Recorder Pro plugin that uploads recorded calls to
//! [OpenMHz](https://openmhz.com), as Trunk Recorder's OpenMHz uploader does.

mod upload;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use trunk_recorder_plugin::{Attempt, CallQueue, ConcludedCall, Host, Manifest, Plugin, QueueOptions, Setup, TalkgroupFilter, Talkgroups, format, topic};

use upload::{Upload, Uploader};

pub const DEFAULT_SERVER: &str = "https://api.openmhz.com";

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    /// Upload server
    ///
    /// Leave this as it is, unless you run your own OpenMHz server.
    #[schemars(url)]
    #[serde(alias = "uploadServer")]
    server: String,
}

impl Default for Config {
    fn default() -> Self {
        Config { server: DEFAULT_SERVER.into() }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", default)]
struct SystemConfig {
    /// API key
    ///
    /// Your system's upload key, from its settings on OpenMHz. Leave it empty
    /// to not upload this system.
    #[schemars(extend("x-secret" = true, "x-required" = true))]
    api_key: String,
    /// Name on OpenMHz
    ///
    /// The system's short name on OpenMHz, if it's different from its short
    /// name here.
    #[serde(alias = "openmhzSystemId")]
    system_name: String,
    // The talkgroups to upload: a menu, and the lists it asks for.
    #[serde(flatten)]
    talkgroups: Talkgroups,
    /// Sites of their own
    ///
    /// A site of this system that is a system of its own on OpenMHz: its calls go there instead.
    sites: Vec<SiteConfig>,
}

/// A site with an OpenMHz system of its own.
#[derive(Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", default)]
struct SiteConfig {
    /// Site
    ///
    /// Its name, as under Sites in Setup.
    site: String,
    /// API key
    ///
    /// Its OpenMHz system's upload key.
    #[schemars(extend("x-secret" = true))]
    api_key: String,
    /// Name on OpenMHz
    ///
    /// Its system's short name on OpenMHz, if it's different from the site's name.
    system_name: String,
}

/// Where a system's calls go.
struct Target {
    /// The queue's lane for its server: its host ("" for OpenMHz's own).
    lane: String,
    uploader: Arc<Uploader>,
    system: String,
    api_key: String,
    /// Sites with an OpenMHz system of their own: (name on OpenMHz, API key), by site name.
    sites: HashMap<String, (String, String)>,
    filter: TalkgroupFilter,
}

impl Target {
    /// Where a call heard on `site` goes: its own OpenMHz system, else the system's.
    fn of(&self, site: &str) -> (&str, &str) {
        self.sites.get(site).map_or((self.system.as_str(), self.api_key.as_str()), |(name, key)| (name.as_str(), key.as_str()))
    }
}

struct OpenMhz {
    queue: CallQueue,
}

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
            ..trunk_recorder_plugin::manifest!()
        }
    }

    fn start(host: Host, setup: Setup<Config, SystemConfig>) -> Result<Self, String> {
        // OpenMHz takes M4A or MP3, not WAV.
        if !setup.has_format(format::M4A) {
            return Err("OpenMHz needs calls as M4A, and there's no M4A encoder on this computer. Install ffmpeg, then start recording again.".into());
        }
        // One uploader for each server (a system can name a server of its own).
        let mut uploaders: HashMap<String, Arc<Uploader>> = HashMap::new();
        let mut targets = HashMap::new();
        for s in &setup.systems {
            let Some(c) = &s.config else { continue };
            if c.api_key.trim().is_empty() {
                continue;
            }
            let server = s.settings.server.trim().trim_end_matches('/').to_string();
            if !(server.starts_with("https://") || server.starts_with("http://")) {
                return Err(format!("The upload server has to be a web address (https://…), not \"{server}\""));
            }
            let uploader = uploaders.entry(server.clone()).or_insert_with(|| Arc::new(Uploader::new(&server))).clone();
            let system = if c.system_name.trim().is_empty() { s.short_name.clone() } else { c.system_name.trim().to_string() };
            let filter = c.talkgroups.filter_for(&host, &s.short_name)?;
            let filtered = if filter.is_empty() { String::new() } else { format!(", talkgroups: {}", filter.describe()) };
            let lane = lane_of(&server);
            let to = if lane.is_empty() { String::new() } else { format!(" to {lane}") };
            host.info(format!("uploading {}{to} as {system} (key …{}){filtered}", s.short_name, last2(&c.api_key)));
            let mut sites = HashMap::new();
            for x in &c.sites {
                let site = x.site.trim();
                if site.is_empty() {
                    return Err(format!("Name the site of {} that has an OpenMHz system of its own.", s.short_name));
                }
                if x.api_key.trim().is_empty() {
                    return Err(format!("Add the OpenMHz API key of {}'s site {site}.", s.short_name));
                }
                let name = if x.system_name.trim().is_empty() { site.to_string() } else { x.system_name.trim().to_string() };
                host.info(format!("uploading {}'s site {site} as {name} (key …{})", s.short_name, last2(&x.api_key)));
                sites.insert(site.to_string(), (name, x.api_key.trim().to_string()));
            }
            targets.insert(s.short_name.clone(), Target { lane, uploader, system, api_key: c.api_key.trim().to_string(), sites, filter });
        }
        if targets.is_empty() {
            return Err("Add your OpenMHz API key to the systems you want to upload.".into());
        }
        // Each server is a lane of the queue: one that's down holds up no other.
        let targets = Arc::new(targets);
        let lanes = targets.clone();
        let opts = QueueOptions { noun: "upload", endpoint: Some("OpenMHz".into()), ..QueueOptions::saved_in(&setup.data_dir) };
        let lane = move |call: &ConcludedCall| lanes.get(&call.call.short_name).map(|t| t.lane.clone()).unwrap_or_default();
        let queue = CallQueue::start_by(host, opts, lane, move |call: &ConcludedCall| {
            // By short name, a system's identity: a call saved for a later run still finds its system.
            let Some(t) = targets.get(&call.call.short_name) else {
                return Attempt::Skip("no OpenMHz API key for this system".into());
            };
            if !t.filter.passes(call.call.talkgroup) {
                return Attempt::Skip(format!("talkgroup {} isn't uploaded (talkgroup filter)", call.call.talkgroup));
            }
            let Some(m4a) = &call.files.m4a else {
                return Attempt::Fail("this call couldn't be encoded as M4A".into());
            };
            let (system, api_key) = t.of(&call.call.site);
            t.uploader.upload(&Upload { system, api_key, call: &call.call, audio: m4a })
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
    use trunk_recorder_plugin::{EXIT_CONFIG, HostMessage, Outcome, State, SystemInfo};

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

    /// A site with an OpenMHz system of its own: its calls go there (named as the site unless given a name), the system's other sites' to the system's.
    #[test]
    fn a_site_of_its_own() {
        let (dir, server) = (testing::temp_dir("omhz"), openmhz());
        let sys = json!({ "apiKey": "good", "sites": [{ "site": "south", "apiKey": "southkey" }, { "site": "east", "apiKey": "eastkey", "systemName": "east-omhz" }] });
        let call = |tg, site: &str| {
            let mut c = testing::call(&dir, "sys1", tg);
            c.call.site = site.into();
            HostMessage::CallConcluded(c)
        };
        testing::run::<OpenMhz>([hello(&dir, server.url(), sys), call(101, "north"), call(102, "south"), call(103, "east")]);
        let mut sent: Vec<(String, String)> =
            server.requests().iter().map(|r| (r.path.clone(), String::from_utf8(r.form_field("api_key").unwrap_or_default()).unwrap())).collect();
        sent.sort();
        assert_eq!(sent, [("/east-omhz/upload".into(), "eastkey".into()), ("/south/upload".into(), "southkey".into()), ("/sys1/upload".into(), "good".into())]);
        // A site of its own needs its name and key.
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "good", "sites": [{ "site": "south" }] }))]);
        assert!(format!("{out:?}").contains("site south"), "{out:?}");
    }

    fn hello(dir: &std::path::Path, server: &str, system: Value) -> HostMessage {
        let mut h = testing::hello(dir, json!({ "server": server }));
        h.systems[0].config = system;
        HostMessage::Hello(h)
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
        let s = trunk_recorder_plugin::describe::<OpenMhz>().system_config.unwrap();
        assert_eq!(s["x-order"], json!(["apiKey", "systemName", "talkgroupMode", "talkgroupAllow", "talkgroupDeny", "sites"]));
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
