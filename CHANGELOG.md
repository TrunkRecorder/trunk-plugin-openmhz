# Changelog

## [0.1.12]

- **Short, stable reasons for skipped and failed calls.** Trunk Recorder Pro after 0.5.1 counts a plugin's
  calls by the reason it gives, on its card: a filtered talkgroup is now **talkgroup filter** (no longer with
  its number; the log says which talkgroup, as an info line), and an audio file that can't be read no longer
  names its path (the recorder's log of the failure names the call).

## [0.1.11]

- Calls with no audio are no longer uploaded (the recorder keeps them when **Keep calls with no
  audio** is on, and OpenMHz showed them as 0-second calls). They are marked skipped.
- **Shortest upload, s**: calls shorter than this stay in your recordings and aren't uploaded. `0`,
  the default, uploads every call that has audio; a system can set its own.
- A busy OpenMHz (`429`/`503`) that says how long to wait (`Retry-After`) isn't sent the call again
  sooner: uploads read its answers as every plugin now does (trunk-recorder-plugin's
  `Attempt::from_status`). Built with trunk-recorder-plugin 0.15.0.

## [0.1.8]

- Four uploads at once for each server, up from two. Every system sending to the same server shares
  them, and at a few thousand calls an hour two couldn't keep up, so calls waited and reached
  OpenMHz late.

## [0.1.7]

- **Talkgroups**: upload every talkgroup, only some or all but some, chosen on each system's
  card. Lists take numbers, ranges (`1000-1099`) and patterns (`507*`); the plugin won't start with
  an entry that's none of them, saying which. Calls on the others are skipped.
- Built with trunk-recorder-plugin 0.8.0 (its `Talkgroups`), for Trunk Recorder Pro v0.2.10.

## [0.1.6]

- Built with trunk-recorder-plugin 0.5.0, for Trunk Recorder Pro v0.2.0: systems are numbered
  from 0 there, conventional ones too, and the call record names every field the recorder writes.

## [0.1.5]

- A system's sites: Trunk Recorder Pro now has systems with several sites, and a
  call heard on more than one is saved once, saying which site it was kept
  from. A site that is a system of its own on OpenMHz can be listed under
  **Sites of their own** with its API key (and its name on OpenMHz); its calls
  go there.
- Built with trunk-recorder-plugin 0.4.0.

## [0.1.4]

- A system can upload to a server of its own (the recorder's per-system override of the server), and each server has its own upload queue, so one that's down doesn't hold up the others.
- Built with trunk-recorder-plugin 0.3.0.

## [0.1.2]

- The API key is marked as needed for each system, so the recorder shows which systems aren't set up for OpenMHz yet.
- Built with trunk-recorder-plugin 0.1.1.

## [0.1.1]

- Releases carry build provenance attestations: proof that GitHub Actions
  built each file from this repository at the tagged commit.

## [0.1.0]

- Uploads recorded calls to OpenMHz as M4A, with the fields Trunk Recorder's
  OpenMHz uploader sends.
- Retries calls OpenMHz couldn't take (the network, the server) after 10 s,
  1 min, 5 min and 15 min, and keeps calls still waiting when recording stops
  for the next start.
- Reads Trunk Recorder's `openmhzSystemId` setting name as well as its own.
