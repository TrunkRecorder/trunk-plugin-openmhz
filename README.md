# OpenMHz for Trunk Recorder Pro

Uploads the calls [Trunk Recorder Pro](https://github.com/TrunkRecorder/trunk-recorder-pro)
records to [OpenMHz](https://openmhz.com), so they can be listened to there.
It does what Trunk Recorder's built-in OpenMHz uploader does.

## What it needs

- **An OpenMHz system** for each system you upload, and its API key. Set one
  up on [openmhz.com](https://openmhz.com).
- **An M4A encoder.** OpenMHz takes M4A, not WAV. On macOS one is built in. On
  Linux and Windows, install [ffmpeg](https://ffmpeg.org) (on Debian and
  Raspberry Pi OS: `sudo apt install ffmpeg`). Without one, the plugin won't
  start, and says so.

## Settings

| Setting | |
|---|---|
| **Upload server** | Leave it as it is, unless you run your own OpenMHz server. |

For each system:

| Setting | |
|---|---|
| **API key** | The system's upload key from OpenMHz. Leave it empty to not upload that system. |
| **Name on OpenMHz** | The system's short name on OpenMHz, if it differs from its short name in the recorder. |
| **Sites of their own** | A site of the system that is a system of its own on OpenMHz: its name (as under Sites in Setup), its OpenMHz API key, and its name on OpenMHz if not the site's. Its calls go there; the system's other sites' go to the system's. |

## What it does with calls

For each recorded call of a system with an API key, it sends OpenMHz the
call's M4A audio and its details: talkgroup, frequency, start and stop time,
length, emergency flag, error counts, and the radios heard on it with their
names. Nothing else, and nowhere else.

If OpenMHz can't be reached, the plugin tries the call again after 10 seconds,
1 minute, 5 minutes and 15 minutes, and shows how many calls are waiting.
Calls still waiting when recording stops are kept, and sent when it starts
again. OpenMHz refusing a call (a wrong API key, an unknown system) isn't
retried: the call is marked failed and the log says why. Calls on talkgroups
your OpenMHz system ignores are marked skipped.

## Sites

In Trunk Recorder Pro a system can have several sites, and a call heard on
more than one is saved once, from the site that heard it best. With one
OpenMHz system for all of them there is nothing to set. If each site has an
OpenMHz system of its own (as when each was a system in Trunk Recorder), list
those sites under **Sites of their own**. A setup from before sites existed
became one system with a site per old system, named as it was and keeping the
first one's OpenMHz settings: add the others' API keys there.

## Coming from Trunk Recorder

Trunk Recorder's settings map across: `uploadServer` is **Upload server**, and
each system's `apiKey` and `openmhzSystemId` are **API key** and **Name on
OpenMHz**. Trunk Recorder systems that are sites of one system (`multiSite`)
are one system here: give each site that has its own OpenMHz system under
**Sites of their own**.

## Building

```sh
cargo build --release
trunk-pro plugin run ./target/release/openmhz ~/TrunkRecorderPro --settings examples/settings.json
```

## License

GPL-3.0-or-later, like Trunk Recorder.
