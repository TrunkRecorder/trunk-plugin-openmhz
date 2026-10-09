# Changelog

## [Unreleased]

- Calls with no audio are no longer uploaded (the recorder keeps them when
  **Keep calls with no audio** is on). They are marked skipped.
- New per-system setting **Shortest call to upload, s**: shorter calls stay in
  your recordings and aren't uploaded. `0`, the default, uploads every call
  that has audio.

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
