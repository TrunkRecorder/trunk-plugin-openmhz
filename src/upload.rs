//! One call to OpenMHz: `POST <server>/<system>/upload`, multipart, the way
//! Trunk Recorder's OpenMHz uploader sends it.

use std::path::Path;
use std::time::Duration;

use serde_json::json;
use trunk_recorder_plugin::{Attempt, CallRecord, Multipart};

pub struct Uploader {
    agent: ureq::Agent,
    server: String,
}

/// What OpenMHz needs to know about a call, besides its audio.
pub struct Upload<'a> {
    pub system: &'a str,
    pub api_key: &'a str,
    pub call: &'a CallRecord,
    pub audio: &'a Path,
}

impl Uploader {
    pub fn new(server: &str) -> Uploader {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            // OpenMHz answers refusals with a 500 and a message: read it.
            .http_status_as_error(false)
            .user_agent(concat!("trunk-lite-openmhz/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Uploader { agent, server: server.trim_end_matches('/').to_string() }
    }

    pub fn upload(&self, u: &Upload) -> Attempt {
        let audio = match std::fs::read(u.audio) {
            Ok(b) => b,
            Err(e) => return Attempt::Fail(format!("can't read {}: {e}", u.audio.display())),
        };
        let name = u.audio.file_name().map_or("call.m4a".into(), |n| n.to_string_lossy().into_owned());
        let (body, content_type) = form(u, &name, &audio);
        let url = format!("{}/{}/upload", self.server, u.system);
        let r = self.agent.post(&url).header("Content-Type", &content_type).send(&body);
        let mut resp = match r {
            Ok(r) => r,
            // Unreachable, timed out, …: try again later.
            Err(e) => return Attempt::Retry(e.to_string()),
        };
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        outcome(status, &text, u.system, &self.server)
    }
}

/// What a response means.
fn outcome(status: u16, text: &str, system: &str, server: &str) -> Attempt {
    if status == 200 {
        let url = if server == crate::DEFAULT_SERVER { format!("https://openmhz.com/system/{system}") } else { String::new() };
        return Attempt::Done { url };
    }
    // OpenMHz's refusals (trunk-server backend/controllers/uploads.js): no use retrying.
    if text.contains("API Keys do not match") {
        return Attempt::Fail(format!("OpenMHz refused the API key for {system}"));
    }
    if text.contains("ShortName does not exist") {
        return Attempt::Fail(format!("OpenMHz has no system named {system}"));
    }
    if text.contains("invalid filename") {
        return Attempt::Fail("OpenMHz refused the file (it takes M4A or MP3)".into());
    }
    if text.contains("Talkgroup does not exist") {
        return Attempt::Skip("OpenMHz ignores talkgroups it doesn't know on this system".into());
    }
    let what = text.trim().lines().next().unwrap_or("").chars().take(200).collect::<String>();
    let why = if what.is_empty() { format!("HTTP {status}") } else { format!("HTTP {status}: {what}") };
    match status {
        // Something else about the request was wrong; it won't get better.
        400..=499 if status != 408 && status != 429 => Attempt::Fail(why),
        _ => Attempt::Retry(why),
    }
}

/// The multipart body, and its content type.
fn form(u: &Upload, file_name: &str, audio: &[u8]) -> (Vec<u8>, String) {
    let c = u.call;
    let sources: Vec<_> = c
        .src_list
        .iter()
        .map(|s| {
            let tag = if s.tag.is_empty() { &s.tag_ota } else { &s.tag };
            json!({ "pos": (s.pos * 100.0).round() / 100.0, "src": s.src, "tag": tag })
        })
        .collect();
    let patches: Vec<i64> =
        c.extra.get("patched_talkgroups").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|v| v.as_i64()).collect()).unwrap_or_default();
    // (Trunk Recorder sends a patch list only when the call was patched.)
    let patches = if patches.len() > 1 { patches } else { Vec::new() };
    let fields = [
        ("freq", c.freq.to_string()),
        ("error_count", c.error_count().to_string()),
        ("spike_count", c.spike_count().to_string()),
        ("start_time", c.start_time.to_string()),
        ("stop_time", c.stop_time.to_string()),
        ("call_length", format!("{:.0}", c.call_length)),
        ("talkgroup_num", c.talkgroup.to_string()),
        ("emergency", (c.emergency as u8).to_string()),
        ("api_key", u.api_key.to_string()),
        ("patch_list", serde_json::to_string(&patches).unwrap_or_default()),
        ("source_list", serde_json::to_string(&sources).unwrap_or_default()),
    ];
    let mut form = Multipart::new().file("call", file_name, "application/octet-stream", audio);
    for (name, value) in fields {
        form = form.text(name, value);
    }
    form.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses() {
        let s = "https://api.openmhz.com";
        assert!(matches!(outcome(200, "", "dcfd", s), Attempt::Done { url } if url == "https://openmhz.com/system/dcfd"));
        assert!(matches!(outcome(500, "API Keys do not match!\n", "dcfd", s), Attempt::Fail(_)));
        assert!(matches!(outcome(500, "ShortName does not exist: dcfd\n", "dcfd", s), Attempt::Fail(_)));
        assert!(matches!(outcome(500, "Talkgroup does not exist, skipping.\n", "dcfd", s), Attempt::Skip(_)));
        assert!(matches!(outcome(500, "Error parsing sourcelist", "dcfd", s), Attempt::Retry(_)));
        assert!(matches!(outcome(502, "", "dcfd", s), Attempt::Retry(_)));
        assert!(matches!(outcome(404, "", "dcfd", s), Attempt::Fail(_)));
        assert!(matches!(outcome(429, "", "dcfd", s), Attempt::Retry(_)));
    }
}
