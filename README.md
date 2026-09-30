# OpenMHz for Trunk Recorder Lite

Uploads the calls [Trunk Recorder Lite](https://github.com/TrunkRecorder/trunk-recorder-lite)
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

## Coming from Trunk Recorder

Trunk Recorder's settings map across: `uploadServer` is **Upload server**, and
each system's `apiKey` and `openmhzSystemId` are **API key** and **Name on
OpenMHz**.

## Building

```sh
cargo build --release
trunk-lite plugin run ./target/release/openmhz ~/TrunkRecorderLite --settings examples/settings.json
```

## License

GPL-3.0-or-later, like Trunk Recorder.
