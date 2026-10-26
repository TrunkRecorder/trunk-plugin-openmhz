# Changelog

## [0.1.7]

- Talkgroup allow and deny lists for each system (**Only these talkgroups**,
  **Not these talkgroups**), with `*` and `?`, as the Rdio Scanner and
  Broadcastify plugins have. A call on a talkgroup they leave out is skipped.

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
