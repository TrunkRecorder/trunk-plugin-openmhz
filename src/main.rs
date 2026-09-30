//! OpenMHz — a Trunk Recorder Lite plugin that uploads recorded calls to
//! [OpenMHz](https://openmhz.com), as Trunk Recorder's OpenMHz uploader does.

mod upload;

use std::collections::HashMap;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use trunk_recorder_plugin::{format, topic, Attempt, CallQueue, ConcludedCall, Host, Manifest, Plugin, QueueOptions, Setup};

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
    #[schemars(extend("x-secret" = true))]
    api_key: String,
    /// Name on OpenMHz
    ///
    /// The system's short name on OpenMHz, if it's different from its short
    /// name here.
    #[serde(alias = "openmhzSystemId")]
    system_name: String,
}

/// Where a system's calls go.
struct Target {
    system: String,
    api_key: String,
}

struct OpenMhz {
    queue: CallQueue,
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
        let server = setup.config.server.trim().to_string();
        if !(server.starts_with("https://") || server.starts_with("http://")) {
            return Err(format!("The upload server has to be a web address (https://…), not \"{server}\""));
        }
        // OpenMHz takes M4A or MP3, not WAV.
        if !setup.has_format(format::M4A) {
            return Err("OpenMHz needs calls as M4A, and there's no M4A encoder on this computer. Install ffmpeg, then start recording again.".into());
        }
        let mut targets = HashMap::new();
        for s in &setup.systems {
            let Some(c) = &s.config else { continue };
            if c.api_key.trim().is_empty() {
                continue;
            }
            let system = if c.system_name.trim().is_empty() { s.short_name.clone() } else { c.system_name.trim().to_string() };
            host.info(format!("uploading {} as {system} (key …{})", s.short_name, last2(&c.api_key)));
            targets.insert(s.index, Target { system, api_key: c.api_key.trim().to_string() });
        }
        if targets.is_empty() {
            return Err("Add your OpenMHz API key to the systems you want to upload.".into());
        }
        let uploader = Uploader::new(&server);
        let opts = QueueOptions { noun: "upload", ..QueueOptions::saved_in(&setup.data_dir) };
        let queue = CallQueue::start(host, opts, move |call: &ConcludedCall| {
            let Some(t) = targets.get(&call.system) else {
                return Attempt::Skip("no OpenMHz API key for this system".into());
            };
            let Some(m4a) = &call.files.m4a else {
                return Attempt::Fail("this call couldn't be encoded as M4A".into());
            };
            uploader.upload(&Upload { system: &t.system, api_key: &t.api_key, call: &call.call, audio: m4a })
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
    use serde_json::{json, Value};
    use trunk_recorder_plugin::testing::{self, MockServer};
    use trunk_recorder_plugin::{HostMessage, Outcome, State, EXIT_CONFIG};

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

    fn hello(dir: &std::path::Path, server: &str, system: Value) -> HostMessage {
        let mut h = testing::hello(dir, json!({ "server": server }));
        h.systems[0].config = system;
        HostMessage::Hello(h)
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
        let out = testing::run::<OpenMhz>([hello(&dir, "http://127.0.0.1:9", json!({ "apiKey": "good" })), HostMessage::CallConcluded(testing::call(&dir, "sys1", 5))]);
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
        let mut call = testing::call(&dir, "sys2", 5);
        call.system = 1;
        let out = testing::run::<OpenMhz>([hello(&dir, server.url(), json!({ "apiKey": "good" })), HostMessage::CallConcluded(call)]);
        assert_eq!(out.results()[0].1, Outcome::Skipped);
        assert!(server.requests().is_empty());
    }
}
