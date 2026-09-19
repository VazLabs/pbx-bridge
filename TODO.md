# TODO

1 → config crate
2 → router crate
3 → Asterisk ARI: connect, answer call, echo audio back
4 → media crate: PCM ↔ Opus pipeline
5 → Telegram userbot: auth + send message (smoke test)
6 → Telegram voice call: initiate/receive (no audio)
7 → Full bridge: bidirectional audio
8 → Hardening: error handling, reconnection, logging

## General

* Otel setup - Tracing, logging at least

## Telegram Crate

[X] create & persist session
  [x] login
  [x] login with 2FA
  [] logout - maybe
  [] verify logged in session matches phone num
[x] build the listen to updates loop
[x] send messages/signals
  [x] request a call
  [] receive a call
  [] decline a call
  [] end a call
[x] development harness binary to mirror bridge like behavior but have direct
   way to send messages into channels
[x] get all control and audio channels from telegram Handle
## Bridge binary

[] CLI command to login and create a session (a one tiime code is needed)
  [] Main loops should exit cleanly if session is not authed, should direct
    user to use the login CLI command and restart the app
[] Health checks
[] coordinate shutdown across crates


## Config

* consider builder pattern for configurations

## Long term TODO
